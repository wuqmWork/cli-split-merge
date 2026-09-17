use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

const EVENT_NAME: &str = "merge-progress";
const DEFAULT_BUFFER_MB: usize = 16;
const UNIT_EPS: f64 = 1e-12;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    pub input_files: Vec<String>,
    pub output_file: String,
    pub read_buffer_mb: Option<usize>,
    pub write_buffer_mb: Option<usize>,
    pub parallel: Option<bool>,
    pub worker_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub input_count: usize,
    pub layer_count: usize,
    pub output_file: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeProgress {
    pub percent: f64,
    pub completed: usize,
    pub total: usize,
    pub current_file: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct LayerSpan {
    input_index: usize,
    layer_value: f64,
    layer_line: Vec<u8>,
    body_start: u64,
    body_end: u64,
}

#[derive(Debug, Clone, Copy)]
struct Dimension {
    xmin: f64,
    ymin: f64,
    zmin: f64,
    xmax: f64,
    ymax: f64,
    zmax: f64,
}

impl Dimension {
    fn union(self, other: Self) -> Self {
        Self {
            xmin: self.xmin.min(other.xmin),
            ymin: self.ymin.min(other.ymin),
            zmin: self.zmin.min(other.zmin),
            xmax: self.xmax.max(other.xmax),
            ymax: self.ymax.max(other.ymax),
            zmax: self.zmax.max(other.zmax),
        }
    }
}

#[derive(Debug)]
struct IndexedFile {
    path: PathBuf,
    header: Vec<u8>,
    preamble: Vec<u8>,
    unit: f64,
    dimension: Option<Dimension>,
    layers: Vec<LayerSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LayerKey(f64);

impl Eq for LayerKey {}
impl PartialOrd for LayerKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for LayerKey {
    fn cmp(&self, other: &Self) -> Ordering { self.0.total_cmp(&other.0) }
}

pub fn merge_cli_files(app: &AppHandle, request: MergeRequest) -> Result<MergeResult, String> {
    let started = Instant::now();
    validate_request(&request)?;
    crate::runtime::log(app, "info", &format!("开始 CLI 合并：{} 个输入文件", request.input_files.len()));

    let output_path = normalize_output_path(&request.output_file);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建输出目录 {}：{e}", parent.display()))?;
    }

    let canonical_output = absolute_for_compare(&output_path);
    for input in &request.input_files {
        if absolute_for_compare(Path::new(input)) == canonical_output {
            return Err("输出文件不能与输入文件相同。".to_string());
        }
    }

    let read_capacity = mb_capacity(request.read_buffer_mb.unwrap_or(DEFAULT_BUFFER_MB));
    let write_capacity = mb_capacity(request.write_buffer_mb.unwrap_or(DEFAULT_BUFFER_MB));
    let total = request.input_files.len();

    // 第一遍只建立每层字节偏移索引，不把整个 CLI 或整层几何加载进内存。
    // 多文件时可并行建立索引，最终仍按输入数组顺序收集，保持同层拼接顺序稳定。
    let parallel = request.parallel.unwrap_or(false) && total > 1;
    let requested_workers = request.worker_count.unwrap_or(0);
    let auto_workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let workers = if requested_workers == 0 { auto_workers } else { requested_workers }.clamp(1, total.max(1));
    let indexed: Vec<IndexedFile> = if parallel {
        crate::runtime::log(app, "info", &format!("启用并行合并索引：{} 个工作线程", workers));
        let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build()
            .map_err(|e| format!("创建并行线程池失败：{e}"))?;
        pool.install(|| {
            request.input_files.par_iter().enumerate().map(|(i, input)| {
                let path = PathBuf::from(input);
                let name = display_name(&path);
                emit_progress(app, 0.0, 0, total, &name, "正在并行扫描层索引");
                index_file(&path, i, read_capacity)
            }).collect::<Result<Vec<_>, String>>()
        })?
    } else {
        let mut values = Vec::with_capacity(total);
        for (i, input) in request.input_files.iter().enumerate() {
            let path = PathBuf::from(input);
            let name = display_name(&path);
            emit_progress(app, (i as f64 / (total as f64 * 2.0)) * 100.0, i, total, &name, "正在扫描层索引");
            values.push(index_file(&path, i, read_capacity)?);
        }
        values
    };

    validate_compatible(&indexed)?;

    let mut by_layer: BTreeMap<LayerKey, Vec<LayerSpan>> = BTreeMap::new();
    for file in &indexed {
        for span in &file.layers {
            by_layer.entry(LayerKey(span.layer_value)).or_default().push(span.clone());
        }
    }
    let layer_count = by_layer.len();
    if layer_count == 0 {
        return Err("输入 CLI 中没有找到 $$LAYER/ 层数据。".to_string());
    }

    // 同层内保持用户选择文件的先后顺序。
    for spans in by_layer.values_mut() {
        spans.sort_by_key(|s| s.input_index);
    }

    let merged_dimension = indexed.iter().filter_map(|f| f.dimension).reduce(Dimension::union);
    let header = rewrite_header(&indexed[0].header, layer_count, merged_dimension)?;

    let temp_path = temp_path_for(&output_path);
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    let result = (|| -> Result<(), String> {
        let out_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp_path)
            .map_err(|e| format!("无法创建临时输出文件 {}：{e}", temp_path.display()))?;
        let mut writer = BufWriter::with_capacity(write_capacity, out_file);

        writer.write_all(&header).map_err(io_err("写入 CLI 头部"))?;
        if !ends_with_newline(&header) {
            writer.write_all(b"\n").map_err(io_err("写入 CLI 头部换行"))?;
        }
        writer.write_all(b"$$GEOMETRYSTART\n").map_err(io_err("写入 GEOMETRYSTART"))?;
        if !indexed[0].preamble.is_empty() {
            writer.write_all(&indexed[0].preamble).map_err(io_err("写入几何前导内容"))?;
            if !ends_with_newline(&indexed[0].preamble) {
                writer.write_all(b"\n").map_err(io_err("写入前导换行"))?;
            }
        }

        let mut sources: Vec<File> = indexed
            .iter()
            .map(|f| File::open(&f.path).map_err(|e| format!("无法重新打开 CLI {}：{e}", f.path.display())))
            .collect::<Result<_, _>>()?;

        let mut copy_buffer = vec![0_u8; 1024 * 1024];
        let layer_total = layer_count.max(1);
        let mut seen_inputs = vec![false; total];
        let mut completed_files = 0usize;
        let mut last_progress_emit = Instant::now();

        for (layer_i, (_key, spans)) in by_layer.iter().enumerate() {
            let layer_line = &spans[0].layer_line;
            writer.write_all(layer_line).map_err(io_err("写入 LAYER"))?;
            if !ends_with_newline(layer_line) {
                writer.write_all(b"\n").map_err(io_err("写入 LAYER 换行"))?;
            }

            for span in spans {
                if !seen_inputs[span.input_index] {
                    seen_inputs[span.input_index] = true;
                    completed_files += 1;
                }
                let current_name = display_name(&indexed[span.input_index].path);
                let p = 50.0 + (layer_i as f64 / layer_total as f64) * 50.0;
                if last_progress_emit.elapsed() >= Duration::from_millis(250) || layer_i + 1 == layer_total {
                    emit_progress(app, p, completed_files.min(total), total, &current_name, "正在按层合并");
                    last_progress_emit = Instant::now();
                }
                copy_range(
                    &mut sources[span.input_index],
                    &mut writer,
                    span.body_start,
                    span.body_end,
                    &mut copy_buffer,
                )?;
            }
        }

        writer.write_all(b"$$GEOMETRYEND\n").map_err(io_err("写入 GEOMETRYEND"))?;
        writer.flush().map_err(|e| format!("刷新输出文件失败：{e}"))?;
        Ok(())
    })();

    if let Err(e) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }

    let backup_path = output_path.with_extension("cli.replace.bak");
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }
    let had_existing = output_path.exists();
    if had_existing {
        fs::rename(&output_path, &backup_path)
            .map_err(|e| format!("无法准备覆盖已有输出文件 {}：{e}", output_path.display()))?;
    }
    if let Err(e) = fs::rename(&temp_path, &output_path) {
        if had_existing && backup_path.exists() {
            let _ = fs::rename(&backup_path, &output_path);
        }
        let _ = fs::remove_file(&temp_path);
        return Err(format!("无法完成输出文件 {}：{e}", output_path.display()));
    }
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }

    emit_progress(
        app,
        100.0,
        total,
        total,
        output_path.file_name().and_then(|s| s.to_str()).unwrap_or("merged.cli"),
        "合并完成",
    );

    let result = MergeResult {
        input_count: total,
        layer_count,
        output_file: output_path.to_string_lossy().into_owned(),
        elapsed_ms: started.elapsed().as_millis(),
    };
    crate::runtime::log(app, "info", &format!("CLI 合并完成：{} ms，{} 层", result.elapsed_ms, result.layer_count));
    if let Ok(payload) = serde_json::to_string(&result) { crate::runtime::write_cache(app, "last_merge", &payload); }
    Ok(result)
}

fn index_file(path: &Path, input_index: usize, capacity: usize) -> Result<IndexedFile, String> {
    if !path.is_file() {
        return Err(format!("输入 CLI 不存在：{}", path.display()));
    }
    let file = File::open(path).map_err(|e| format!("无法打开 CLI {}：{e}", path.display()))?;
    let mut reader = BufReader::with_capacity(capacity, file);
    let mut pos = 0_u64;
    let mut line = Vec::with_capacity(4096);
    let mut header = Vec::new();
    let mut preamble = Vec::new();
    let mut unit = 1.0_f64;
    let mut dimension = None;
    let mut geometry_started = false;
    let mut geometry_finished = false;
    let mut saw_layer = false;
    let mut current: Option<LayerSpan> = None;
    let mut layers = Vec::new();

    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)
            .map_err(|e| format!("读取 CLI {} 失败：{e}", path.display()))?;
        if n == 0 { break; }
        let line_start = pos;
        pos += n as u64;
        let token = trim_ascii(&line);

        if !geometry_started {
            if eq_ascii_ci(token, b"$$GEOMETRYSTART") {
                geometry_started = true;
                continue;
            }
            if starts_ascii_ci(token, b"$$UNITS/") {
                unit = parse_units_line(token)?;
            } else if starts_ascii_ci(token, b"$$DIMENSION/") {
                dimension = Some(parse_dimension_line(token)?);
            }
            header.extend_from_slice(&line);
            continue;
        }

        if eq_ascii_ci(token, b"$$GEOMETRYEND") {
            if let Some(mut span) = current.take() {
                span.body_end = line_start;
                layers.push(span);
            }
            geometry_finished = true;
            break;
        }

        if starts_ascii_ci(token, b"$$LAYER/") {
            if let Some(mut span) = current.take() {
                span.body_end = line_start;
                layers.push(span);
            }
            let layer_value = parse_layer_value(token)
                .map_err(|e| format!("{}：{}", path.display(), e))?;
            saw_layer = true;
            current = Some(LayerSpan {
                input_index,
                layer_value,
                layer_line: line.clone(),
                body_start: pos,
                body_end: pos,
            });
            continue;
        }

        if !saw_layer {
            preamble.extend_from_slice(&line);
        }
    }

    if !geometry_started {
        return Err(format!("CLI 缺少 $$GEOMETRYSTART：{}", path.display()));
    }
    if !geometry_finished {
        return Err(format!("CLI 缺少 $$GEOMETRYEND：{}", path.display()));
    }

    Ok(IndexedFile { path: path.to_path_buf(), header, preamble, unit, dimension, layers })
}

fn validate_request(request: &MergeRequest) -> Result<(), String> {
    if request.input_files.len() < 2 {
        return Err("CLI 合并至少需要选择两个输入文件。".to_string());
    }
    if request.output_file.trim().is_empty() {
        return Err("请选择输出文件。".to_string());
    }

    let mut seen = HashSet::new();
    for input in &request.input_files {
        let path = Path::new(input);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("cli") {
            return Err(format!("仅支持 .cli 输入文件：{input}"));
        }
        let normalized = absolute_for_compare(path);
        if !seen.insert(normalized.clone()) {
            return Err(format!("输入文件重复：{}", normalized.display()));
        }
    }
    Ok(())
}

fn validate_compatible(files: &[IndexedFile]) -> Result<(), String> {
    let expected = files.first().map(|f| f.unit).unwrap_or(1.0);
    for file in files.iter().skip(1) {
        let scale = expected.abs().max(file.unit.abs()).max(1.0);
        if (file.unit - expected).abs() > UNIT_EPS * scale {
            return Err(format!(
                "CLI 单位不一致，无法直接按层合并：{}（期望 {}，实际 {}）",
                file.path.display(), expected, file.unit
            ));
        }
    }
    Ok(())
}

fn rewrite_header(header: &[u8], layer_count: usize, dimension: Option<Dimension>) -> Result<Vec<u8>, String> {
    let mut reader = BufReader::new(header);
    let mut line = Vec::new();
    let mut out = Vec::with_capacity(header.len() + 128);
    let mut saw_layers = false;
    let mut saw_dimension = false;
    let mut inserted_missing = false;

    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).map_err(|e| format!("重写 CLI 头部失败：{e}"))?;
        if n == 0 { break; }
        let token = trim_ascii(&line);

        // 若原头部缺少元数据，必须在 HEADEREND 之前补入，不能追加到头部结束标记之后。
        if eq_ascii_ci(token, b"$$HEADEREND") && !inserted_missing {
            if !saw_layers {
                writeln!(&mut out, "$$LAYERS/{layer_count}").map_err(|e| format!("写入 LAYERS 失败：{e}"))?;
                saw_layers = true;
            }
            if !saw_dimension {
                if let Some(d) = dimension {
                    write_dimension_line(&mut out, d)?;
                    saw_dimension = true;
                }
            }
            inserted_missing = true;
        }

        if starts_ascii_ci(token, b"$$LAYERS/") {
            writeln!(&mut out, "$$LAYERS/{layer_count}").map_err(|e| format!("重写 LAYERS 失败：{e}"))?;
            saw_layers = true;
        } else if starts_ascii_ci(token, b"$$DIMENSION/") {
            if let Some(d) = dimension {
                write_dimension_line(&mut out, d)?;
            } else {
                out.extend_from_slice(&line);
            }
            saw_dimension = true;
        } else {
            out.extend_from_slice(&line);
        }
    }

    if !inserted_missing {
        if !saw_layers {
            writeln!(&mut out, "$$LAYERS/{layer_count}").map_err(|e| format!("写入 LAYERS 失败：{e}"))?;
        }
        if !saw_dimension {
            if let Some(d) = dimension {
                write_dimension_line(&mut out, d)?;
            }
        }
    }
    Ok(out)
}

fn write_dimension_line(out: &mut Vec<u8>, d: Dimension) -> Result<(), String> {
    writeln!(out, "$$DIMENSION/{},{},{},{},{},{}", fmt_num(d.xmin), fmt_num(d.ymin), fmt_num(d.zmin), fmt_num(d.xmax), fmt_num(d.ymax), fmt_num(d.zmax))
        .map_err(|e| format!("重写 DIMENSION 失败：{e}"))
}

fn fmt_num(v: f64) -> String {
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(v);
    text.strip_suffix(".0").unwrap_or(text).to_string()
}

fn parse_units_line(token: &[u8]) -> Result<f64, String> {
    let slash = token.iter().position(|&b| b == b'/').ok_or_else(|| "UNITS 行格式错误".to_string())?;
    let value = trim_ascii(&token[slash + 1..]);
    if value.is_empty() { return Ok(1.0); }
    let parsed = lexical_core::parse::<f64>(value)
        .map_err(|_| format!("无法解析 UNITS：{}", String::from_utf8_lossy(value)))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(format!("UNITS 必须为正有限数值：{}", String::from_utf8_lossy(value)));
    }
    Ok(parsed)
}

fn parse_dimension_line(token: &[u8]) -> Result<Dimension, String> {
    let slash = token.iter().position(|&b| b == b'/').ok_or_else(|| "DIMENSION 行格式错误".to_string())?;
    let payload = trim_ascii(&token[slash + 1..]);
    let text = std::str::from_utf8(payload).map_err(|_| "DIMENSION 不是有效 ASCII 数值".to_string())?;
    let values: Vec<f64> = text
        .split(',')
        .map(|v| lexical_core::parse::<f64>(v.trim().as_bytes()).map_err(|_| format!("无法解析 DIMENSION 数值：{v}")))
        .collect::<Result<_, _>>()?;
    if values.len() != 6 || values.iter().any(|v| !v.is_finite()) {
        return Err("DIMENSION 必须包含 6 个有限数值".to_string());
    }
    Ok(Dimension { xmin: values[0], ymin: values[1], zmin: values[2], xmax: values[3], ymax: values[4], zmax: values[5] })
}

fn parse_layer_value(token: &[u8]) -> Result<f64, String> {
    let slash = token.iter().position(|&b| b == b'/')
        .ok_or_else(|| "LAYER 行格式错误".to_string())?;
    let value = trim_ascii(&token[slash + 1..]);
    lexical_core::parse::<f64>(value)
        .map_err(|_| format!("无法解析层高：{}", String::from_utf8_lossy(value)))
}

fn copy_range(
    file: &mut File,
    writer: &mut BufWriter<File>,
    start: u64,
    end: u64,
    buffer: &mut [u8],
) -> Result<(), String> {
    if end <= start { return Ok(()); }
    file.seek(SeekFrom::Start(start)).map_err(|e| format!("定位源 CLI 失败：{e}"))?;
    let mut remain = end - start;
    while remain > 0 {
        let take = remain.min(buffer.len() as u64) as usize;
        let n = file.read(&mut buffer[..take]).map_err(|e| format!("读取层数据失败：{e}"))?;
        if n == 0 { return Err("源 CLI 在预期层数据结束前意外结束。".to_string()); }
        writer.write_all(&buffer[..n]).map_err(|e| format!("写入合并层数据失败：{e}"))?;
        remain -= n as u64;
    }
    Ok(())
}

fn normalize_output_path(raw: &str) -> PathBuf {
    let path = PathBuf::from(raw.trim());
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("cli") => path,
        _ => PathBuf::from(format!("{}.cli", path.to_string_lossy())),
    }
}

fn temp_path_for(output: &Path) -> PathBuf {
    let name = output.file_name().and_then(|s| s.to_str()).unwrap_or("merged.cli");
    output.with_file_name(format!(".{name}.merge.tmp"))
}

fn absolute_for_compare(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() { path.to_path_buf() }
        else { std::env::current_dir().unwrap_or_default().join(path) }
    })
}

fn display_name(path: &Path) -> String {
    path.file_name().and_then(|s| s.to_str()).unwrap_or_else(|| path.to_str().unwrap_or("CLI")).to_string()
}

fn mb_capacity(mb: usize) -> usize { mb.clamp(1, 256) * 1024 * 1024 }

fn ends_with_newline(bytes: &[u8]) -> bool {
    bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r')
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { bytes = &bytes[3..]; }
    while bytes.first().is_some_and(|b| b.is_ascii_whitespace()) { bytes = &bytes[1..]; }
    while bytes.last().is_some_and(|b| b.is_ascii_whitespace()) { bytes = &bytes[..bytes.len()-1]; }
    bytes
}

fn eq_ascii_ci(a: &[u8], b: &[u8]) -> bool { a.eq_ignore_ascii_case(b) }
fn starts_ascii_ci(a: &[u8], prefix: &[u8]) -> bool {
    a.len() >= prefix.len() && a[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn emit_progress(app: &AppHandle, percent: f64, completed: usize, total: usize, current_file: &str, message: &str) {
    let _ = app.emit(EVENT_NAME, MergeProgress {
        percent: percent.clamp(0.0, 100.0),
        completed,
        total,
        current_file: current_file.to_string(),
        message: message.to_string(),
    });
}

fn io_err(context: &'static str) -> impl FnOnce(std::io::Error) -> String {
    move |e| format!("{context}失败：{e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_layer() {
        assert_eq!(parse_layer_value(b"$$LAYER/12.5").unwrap(), 12.5);
        assert_eq!(parse_layer_value(b"  $$layer/-0.125  ").unwrap(), -0.125);
    }

    #[test]
    fn output_extension_is_added() {
        assert!(normalize_output_path("abc").to_string_lossy().ends_with("abc.cli"));
        assert!(normalize_output_path("abc.CLI").to_string_lossy().ends_with("abc.CLI"));
    }

    #[test]
    fn units_compare_numerically() {
        assert_eq!(parse_units_line(b"$$UNITS/1").unwrap(), parse_units_line(b"$$UNITS/1.0").unwrap());
    }

    #[test]
    fn dimension_union_is_correct() {
        let a = Dimension { xmin: 0.0, ymin: 1.0, zmin: 2.0, xmax: 3.0, ymax: 4.0, zmax: 5.0 };
        let b = Dimension { xmin: -1.0, ymin: 2.0, zmin: 0.0, xmax: 10.0, ymax: 3.0, zmax: 8.0 };
        let u = a.union(b);
        assert_eq!((u.xmin, u.ymin, u.zmin, u.xmax, u.ymax, u.zmax), (-1.0, 1.0, 0.0, 10.0, 4.0, 8.0));
    }

    #[test]
    fn rewrites_layers_before_header_end() {
        let header = b"$$HEADERSTART
$$UNITS/1
$$LAYERS/2
$$DIMENSION/0,0,0,1,1,1
$$HEADEREND
";
        let d = Dimension { xmin: -2.0, ymin: -3.0, zmin: 0.0, xmax: 4.0, ymax: 5.0, zmax: 6.0 };
        let out = rewrite_header(header, 7, Some(d)).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("$$LAYERS/7"));
        assert!(text.contains("$$DIMENSION/-2,-3,0,4,5,6"));
        assert!(text.find("$$LAYERS/7").unwrap() < text.find("$$HEADEREND").unwrap());
    }
}
