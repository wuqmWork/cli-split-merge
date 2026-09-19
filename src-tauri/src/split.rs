use crate::parameter::MirrorTransform;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, AtomicUsize, Ordering as AtomicOrdering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

const REGION_COUNT: usize = 24;
const EPS: f64 = 1e-9;
const EVENT_NAME: &str = "split-progress";
// 进度上报节流。60ms 兼顾流畅与开销：快速任务也能出现过渡帧，超大文件每秒最多约 17 次 IPC。
const PROGRESS_THROTTLE: Duration = Duration::from_millis(60);

// v14.1: CLI 分割属于重 I/O 工作负载，不能按 CPU 核心数无限展开文件级并行，
// 但也不必过度保守：24 路输出本身写放大有限，4-6 路文件并发可让 NVMe 吃满。
const DEFAULT_SPLIT_WRITE_BUFFER_MB: usize = 2;

// 自动模式下最多同时处理 6 个 CLI（大文件时按阈值自动降档）。
const AUTO_MAX_FILE_WORKERS: usize = 6;

// 用户手动指定 worker 时仍做安全上限，避免误设 16/32 导致 24 路输出成倍展开。
const MANUAL_MAX_FILE_WORKERS: usize = 8;

// 单 CLI 内部 HATCHES 几何并行上限。
// 几何并行只在 total == 1 时启用，禁止和多文件并行叠加。
const AUTO_MAX_GEOMETRY_WORKERS: usize = 6;
const MANUAL_MAX_GEOMETRY_WORKERS: usize = 8;

const GB: u64 = 1024 * 1024 * 1024;
const LARGE_FILE_THRESHOLD: u64 = 16 * GB;
const HUGE_FILE_THRESHOLD: u64 = 32 * GB;

#[derive(Debug, Clone, Copy)]
struct Region {
    name: &'static str,
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
    folder_id: u8,
}

// 坐标范围位于全局 C3 坐标系；folder_id 是设备的物理编号目录。
const REGIONS: [Region; REGION_COUNT] = [
    Region {
        name: "A0",
        xmin: -1430.0,
        xmax: -1170.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 22,
    },
    Region {
        name: "A1",
        xmin: -1430.0,
        xmax: -1170.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 23,
    },
    Region {
        name: "A2",
        xmin: -1170.0,
        xmax: -910.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 4,
    },
    Region {
        name: "A3",
        xmin: -1170.0,
        xmax: -910.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 1,
    },
    Region {
        name: "B0",
        xmin: -910.0,
        xmax: -650.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 3,
    },
    Region {
        name: "B1",
        xmin: -910.0,
        xmax: -650.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 12,
    },
    Region {
        name: "B2",
        xmin: -650.0,
        xmax: -390.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 7,
    },
    Region {
        name: "B3",
        xmin: -650.0,
        xmax: -390.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 14,
    },
    Region {
        name: "C0",
        xmin: -390.0,
        xmax: -130.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 15,
    },
    Region {
        name: "C1",
        xmin: -390.0,
        xmax: -130.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 11,
    },
    Region {
        name: "C2",
        xmin: -130.0,
        xmax: 130.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 10,
    },
    Region {
        name: "C3",
        xmin: -130.0,
        xmax: 130.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 6,
    },
    Region {
        name: "D0",
        xmin: 130.0,
        xmax: 390.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 19,
    },
    Region {
        name: "D1",
        xmin: 130.0,
        xmax: 390.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 2,
    },
    Region {
        name: "D2",
        xmin: 390.0,
        xmax: 650.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 21,
    },
    Region {
        name: "D3",
        xmin: 390.0,
        xmax: 650.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 24,
    },
    Region {
        name: "E0",
        xmin: 650.0,
        xmax: 910.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 16,
    },
    Region {
        name: "E1",
        xmin: 650.0,
        xmax: 910.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 9,
    },
    Region {
        name: "E2",
        xmin: 910.0,
        xmax: 1170.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 13,
    },
    Region {
        name: "E3",
        xmin: 910.0,
        xmax: 1170.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 8,
    },
    Region {
        name: "F0",
        xmin: 1170.0,
        xmax: 1430.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 20,
    },
    Region {
        name: "F1",
        xmin: 1170.0,
        xmax: 1430.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 5,
    },
    Region {
        name: "F2",
        xmin: 1430.0,
        xmax: 1690.0,
        ymin: -420.0,
        ymax: -140.0,
        folder_id: 17,
    },
    Region {
        name: "F3",
        xmin: 1430.0,
        xmax: 1690.0,
        ymin: -140.0,
        ymax: 140.0,
        folder_id: 18,
    },
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitRequest {
    pub input_files: Vec<String>,
    pub output_dir: String,
    pub naming_template: String,
    // v14: split 的 mmap 路径不再使用读取缓冲；保留字段只为前后端 IPC 反序列化兼容。
    #[allow(dead_code)]
    pub read_buffer_mb: Option<usize>,
    pub write_buffer_mb: Option<usize>,
    pub parallel: Option<bool>,
    pub worker_count: Option<usize>,
    pub overlap_mm: Option<f64>,
    pub transforms: Vec<MirrorTransform>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitResult {
    pub input_count: usize,
    pub output_count: usize,
    pub output_dir: String,
    pub elapsed_ms: u128,
    /// 校验/解析失败被跳过的输入文件（文件名），为空表示全部成功。
    #[serde(default)]
    pub failed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitProgress {
    pub percent: f64,
    pub completed: usize,
    pub total: usize,
    pub current_file: String,
    pub current_file_percent: f64,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
struct Transform2D {
    a: f64,
    b: f64,
    tx: f64,
    ty: f64,
}

impl Transform2D {
    #[inline(always)]
    fn apply(self, p: (f64, f64)) -> (f64, f64) {
        (
            self.a * p.0 - self.b * p.1 + self.tx,
            self.b * p.0 + self.a * p.1 + self.ty,
        )
    }
}

struct OutputFile {
    final_path: PathBuf,
    temp_path: PathBuf,
    writer: BufWriter<File>,
    next_id: u64,
    // 热路径复用的单实体输出缓冲。先在内存中拼成完整 ASCII 行，再一次 write_all，
    // 避免每个坐标/逗号都穿过 BufWriter::write_all。
    line_buffer: Vec<u8>,
}

/// 单区域 POLYLINE 跨记录链式合并器（对齐参考实现 cli_splitter_fast.c 的 PendingPolyline）。
///
/// 裁剪后共点连续的折线片段拼成一条输出，减少振镜跳段；
/// 遇 HATCHES/LAYER 或几何流结束时通过 finish 落盘最后一条链。
/// 落盘的合并线 kind 固定为 0（与参考实现一致）。
struct PolylineChain {
    points: Vec<(f64, f64)>,
}

impl PolylineChain {
    fn new() -> Self {
        Self { points: Vec::new() }
    }

    /// 并入一段裁剪后的折线片段（调用方保证 piece.len() >= 2）。
    /// 返回 Some(chain) 表示链在此断开、需要先把返回的链写出。
    fn push(&mut self, piece: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
        let mut flushed = None;
        if let Some(&last) = self.points.last() {
            // 接续判定与参考实现一致：裁剪后 mm 空间 1e-6 容差。
            let continuous =
                (last.0 - piece[0].0).abs() <= 1e-6 && (last.1 - piece[0].1).abs() <= 1e-6;
            if !continuous {
                flushed = if self.points.len() >= 2 {
                    Some(std::mem::take(&mut self.points))
                } else {
                    self.points.clear();
                    None
                };
            }
        }
        if self.points.is_empty() {
            self.points.extend_from_slice(piece);
        } else {
            // 连续接续：跳过与上段末点重合的共享点。
            self.points.extend(piece[1..].iter().copied());
        }
        flushed
    }

    /// 几何流中断（HATCHES/LAYER）或结束时调用，返回待写出的最后一条链。
    fn finish(&mut self) -> Option<Vec<(f64, f64)>> {
        if self.points.len() >= 2 {
            Some(std::mem::take(&mut self.points))
        } else {
            self.points.clear();
            None
        }
    }
}

/// 写出一条合并完成的 POLYLINE（kind=0，ID 顺序递增，同参考实现）。
fn write_chain(
    chain: Option<Vec<(f64, f64)>>,
    out: &mut OutputFile,
    local: Transform2D,
    unit: f64,
) -> Result<(), String> {
    if let Some(points) = chain {
        let id = out.next_id;
        out.next_id += 1;
        write_polyline_buffered(out, id, 0, &points, local, unit)?;
    }
    Ok(())
}

#[derive(Debug)]
struct Polyline {
    points: Vec<(f64, f64)>,
}

type HatchSegment = (f64, f64, f64, f64);

const X_MIN: f64 = -1430.0;
const FIELD_X_MAX: f64 = 1690.0;
const FIELD_Y_MIN: f64 = -420.0;
const FIELD_Y_MAX: f64 = 140.0;
const CELL_W: f64 = 260.0;
const COLS: usize = 12;
const TOP_BY_COL: [usize; COLS] = [0, 3, 4, 7, 8, 11, 12, 15, 16, 19, 20, 23];
const BOTTOM_BY_COL: [usize; COLS] = [1, 2, 5, 6, 9, 10, 13, 14, 17, 18, 21, 22];

pub fn split_cli_files(app: &AppHandle, request: SplitRequest) -> Result<SplitResult, String> {
    let started = Instant::now();
    validate_request(&request)?;
    crate::runtime::log(
        app,
        "info",
        &format!(
            "开始 CLI 分割：{} 个输入文件，重叠区域 {:.3} mm",
            request.input_files.len(),
            request.overlap_mm.unwrap_or(2.0)
        ),
    );

    let output_root = PathBuf::from(&request.output_dir);
    fs::create_dir_all(&output_root)
        .map_err(|e| format!("无法创建输出目录 {}：{e}", output_root.display()))?;

    let transform_table = build_transform_table(&request.transforms)?;
    let overlap_mm = request.overlap_mm.unwrap_or(2.0);
    validate_output_collisions(&request, &output_root)?;
    for region in REGIONS {
        fs::create_dir_all(output_root.join(region.folder_id.to_string()))
            .map_err(|e| format!("无法创建输出目录 {}：{e}", region.folder_id))?;
    }

    let total = request.input_files.len();

    // v14: split 已经使用 mmap，read_buffer_mb 对分割热路径没有实际作用。
    // 为兼容前端请求字段，SplitRequest 暂时保留 read_buffer_mb，但后端不再分配读取缓冲。
    let write_capacity = mb_capacity(
        request
            .write_buffer_mb
            .unwrap_or(DEFAULT_SPLIT_WRITE_BUFFER_MB),
        DEFAULT_SPLIT_WRITE_BUFFER_MB,
    );

    let parallel_enabled = request.parallel.unwrap_or(false);
    let requested_workers = request.worker_count.unwrap_or(0);

    let file_workers = if parallel_enabled && total > 1 {
        determine_file_workers(&request.input_files, requested_workers)
    } else {
        1
    };

    let geometry_workers = if parallel_enabled && total == 1 {
        determine_geometry_workers(requested_workers)
    } else {
        1
    };

    let file_level_parallel = parallel_enabled && total > 1 && file_workers > 1;
    let geometry_parallel = parallel_enabled && total == 1 && geometry_workers > 1;

    let largest_file = largest_input_file_size(&request.input_files);

    crate::runtime::log(
        app,
        "info",
        &format!(
            "v14.1 分割调度：文件数={}，最大文件={:.2} GB，文件并发={}，单文件几何线程={}，写缓冲={} MB",
            total,
            largest_file as f64 / GB as f64,
            file_workers,
            geometry_workers,
            request
                .write_buffer_mb
                .unwrap_or(DEFAULT_SPLIT_WRITE_BUFFER_MB),
        ),
    );

    // 按文件字节大小加权聚合进度：大文件占多少比例就占多少进度条，
    // 避免"小文件秒完把进度条推高、大文件长期卡在 9x%"的失真。
    let file_sizes: Vec<u64> = request
        .input_files
        .iter()
        .map(|path| fs::metadata(path).map(|m| m.len()).unwrap_or(0))
        .collect();
    let tracker = ProgressTracker::new(file_sizes);

    // 单文件失败只跳过并记录，不中止整个任务；结束后统一汇总报告。
    let process_one = |file_index: usize, input: &String| -> Result<(), String> {
        let input_path = PathBuf::from(input);
        let current_name = input_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(input)
            .to_string();
        emit_progress(
            app,
            &tracker,
            total,
            file_index,
            &current_name,
            format!("正在分割 {current_name}"),
        );
        let outcome = split_one_file(
            app,
            &input_path,
            &output_root,
            file_index + 1,
            total,
            &request.naming_template,
            &transform_table,
            overlap_mm,
            write_capacity,
            geometry_parallel,
            geometry_workers,
            &tracker,
        )
        .map_err(|error| {
            if let Some(stem) = input_path.file_stem().and_then(|v| v.to_str()) {
                cleanup_expected_temp_paths(
                    &output_root,
                    stem,
                    file_index + 1,
                    &request.naming_template,
                );
            }
            format!("{error}（输入文件：{current_name}）")
        });
        // 无论成功还是跳过，都把该文件槽位闭环，避免进度条与计数卡在中途。
        let finished = tracker.finish_file(file_index);
        match outcome {
            Ok(()) => {
                emit_progress(
                    app,
                    &tracker,
                    total,
                    file_index,
                    &current_name,
                    format!("已完成 {current_name}（{finished}/{total}）"),
                );
                Ok(())
            }
            Err(error) => {
                emit_progress(
                    app,
                    &tracker,
                    total,
                    file_index,
                    &current_name,
                    format!("已跳过 {current_name}（{finished}/{total}）"),
                );
                Err(error)
            }
        }
    };

    // 收集每个文件的结果：失败不中断其他文件，全部跑完后汇总。
    let results: Vec<Result<(), String>> = if file_level_parallel {
        crate::runtime::log(
            app,
            "info",
            &format!("启用文件级并行分割：{} 个文件 worker", file_workers),
        );
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(file_workers)
            .build()
            .map_err(|e| format!("创建文件级并行线程池失败：{e}"))?;
        pool.install(|| {
            request
                .input_files
                .par_iter()
                .enumerate()
                .map(|(i, input)| process_one(i, input))
                .collect::<Vec<_>>()
        })
    } else {
        request
            .input_files
            .iter()
            .enumerate()
            .map(|(i, input)| process_one(i, input))
            .collect()
    };

    // 汇总失败文件（保持输入顺序，便于用户对照）。
    let failed_files: Vec<String> = request
        .input_files
        .iter()
        .zip(&results)
        .filter_map(|(input, result)| {
            result.as_ref().err().map(|error| {
                crate::runtime::log(app, "error", &format!("CLI 分割失败：{error}"));
                input
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(input)
                    .to_string()
            })
        })
        .collect();

    if failed_files.len() == total {
        let error = format!(
            "全部 {} 个输入文件均处理失败（首个错误：{}）",
            total,
            results
                .iter()
                .find_map(|r| r.as_ref().err().cloned())
                .unwrap_or_default()
        );
        crate::runtime::log(app, "error", &format!("CLI 分割失败：{error}"));
        return Err(error);
    }
    if !failed_files.is_empty() {
        crate::runtime::log(
            app,
            "warn",
            &format!(
                "跳过 {} 个失败文件：{}",
                failed_files.len(),
                failed_files.join("、")
            ),
        );
    }

    let success_count = total - failed_files.len();
    let result = SplitResult {
        input_count: total,
        output_count: success_count * REGION_COUNT,
        output_dir: output_root.to_string_lossy().into_owned(),
        elapsed_ms: started.elapsed().as_millis(),
        failed_files,
    };
    crate::runtime::log(
        app,
        "info",
        &format!(
            "CLI 分割完成：成功 {}/{} 个文件，{} ms",
            success_count, total, result.elapsed_ms
        ),
    );
    if let Ok(payload) = serde_json::to_string(&result) {
        crate::runtime::write_cache(app, "last_split", &payload);
    }
    Ok(result)
}

fn split_one_file(
    app: &AppHandle,
    input_path: &Path,
    output_root: &Path,
    index: usize,
    total_files: usize,
    naming_template: &str,
    transforms: &[Transform2D; REGION_COUNT],
    overlap_mm: f64,
    write_capacity: usize,
    intra_file_parallel: bool,
    geometry_worker_count: usize,
    // 全任务共享的进度聚合器。并行下文件完成顺序与序号无关，必须读共享状态而非用 index 推算。
    tracker: &ProgressTracker,
) -> Result<(), String> {
    let slot = index - 1;
    if !input_path.is_file() {
        return Err(format!("输入 CLI 不存在：{}", input_path.display()));
    }

    let stem = input_path
        .file_stem()
        .and_then(|v| v.to_str())
        .ok_or_else(|| format!("无法读取文件名：{}", input_path.display()))?;
    let file_name = input_path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(stem)
        .to_string();
    let input_size = fs::metadata(input_path).map(|m| m.len()).unwrap_or(0);

    // 单个 CLI 的实际分割区域在整个处理生命周期内保持不变。
    // 0 mm 时直接借用静态 REGIONS：不复制、不重算。
    // >0 mm 时仅预计算一次 24 个重叠 Region，后续候选筛选、裁剪和 DIMENSION 全程复用。
    let effective_regions;
    let regions: &[Region; REGION_COUNT] = if overlap_mm == 0.0 {
        &REGIONS
    } else {
        effective_regions = build_effective_regions(overlap_mm);
        &effective_regions
    };

    // 单文件并行线程池每个 CLI 只创建一次，避免每条 HATCHES 重建线程池。
    let intra_pool = if intra_file_parallel {
        crate::runtime::log(
            app,
            "info",
            &format!(
                "单 CLI 模式启用几何并行：{} 个线程（不会同时启用文件级并行）",
                geometry_worker_count
            ),
        );

        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(geometry_worker_count)
                .build()
                .map_err(|e| format!("创建单文件几何线程池失败：{e}"))?,
        )
    } else {
        None
    };

    let file = File::open(input_path)
        .map_err(|e| format!("无法打开 CLI {}：{e}", input_path.display()))?;
    // v13: 64 位桌面环境使用只读 mmap + memchr 扫描行边界。
    // 不再 read_until 把每一行复制进 Vec，超长 HATCHES 行直接以 &[u8] 切片解析。
    let mapped = unsafe { memmap2::MmapOptions::new().map(&file) }
        .map_err(|e| format!("内存映射 CLI {} 失败：{e}", input_path.display()))?;

    let mut header: Vec<String> = Vec::new();
    let mut preamble: Vec<String> = Vec::new();
    let mut geometry_started = false;
    let mut geometry_finished = false;
    let mut first_geometry_seen = false;
    let mut outputs: Option<Vec<OutputFile>> = None;
    let mut unit = 1.0_f64;
    let mut last_emit = Instant::now();
    let mut line_start = 0usize;
    // HATCHES 是大文件最常见的热点。24 个区域的容器只创建一次并反复 clear，
    // 避免每条 HATCHES 都分配 24 组 Vec。
    let mut hatch_scratch: Vec<Vec<HatchSegment>> = (0..REGION_COUNT).map(|_| Vec::new()).collect();
    // POLYLINE 跨记录链式合并缓冲：每区域一条链，遇 HATCHES/LAYER 或几何流结束时落盘。
    let mut pending: Vec<PolylineChain> = (0..REGION_COUNT).map(|_| PolylineChain::new()).collect();

    loop {
        if line_start >= mapped.len() {
            break;
        }
        let remaining = &mapped[line_start..];
        let line_end = memchr::memchr(b'\n', remaining)
            .map(|offset| line_start + offset + 1)
            .unwrap_or(mapped.len());
        let line_bytes = &mapped[line_start..line_end];
        line_start = line_end;
        // v13 热路径：几何区完全按原始字节处理。CLI 几何关键字/数值本身是 ASCII，
        // 因此无需为每一行做 UTF-8/lossy -> String 分配；GBK LABEL/注释也能原样透传。
        let normalized_bytes = normalize_cli_line_bytes(line_bytes);
        let token_bytes = trim_ascii_bytes(normalized_bytes);

        if !geometry_started {
            if eq_ascii_ci_bytes(token_bytes, b"$$GEOMETRYSTART") {
                geometry_started = true;
                unit = parse_units(&header)?;
            } else {
                // 头部通常只有几十行，保留 String 便于 LABEL/DIMENSION 重写；此处不在性能热点。
                header.push(String::from_utf8_lossy(normalized_bytes).into_owned());
            }
            continue;
        }

        if eq_ascii_ci_bytes(token_bytes, b"$$GEOMETRYEND") {
            geometry_finished = true;
            break;
        }

        if starts_with_ascii_ci_bytes(token_bytes, b"$$POLYLINE/") {
            first_geometry_seen = true;
            if outputs.is_none() {
                outputs = Some(open_outputs(
                    output_root,
                    stem,
                    index,
                    naming_template,
                    &header,
                    &preamble,
                    transforms,
                    regions,
                    unit,
                    write_capacity,
                )?);
            }
            let poly = parse_polyline_bytes(normalized_bytes, unit)?;
            let bbox = points_bbox(&poly.points);
            let outs = outputs.as_mut().expect("outputs opened");

            let (candidate_indices, candidate_count) =
                candidate_regions_for_bbox(bbox, regions, overlap_mm);
            for &r_index in &candidate_indices[..candidate_count] {
                let region = regions[r_index];
                let pieces = clip_polyline(&poly.points, region, overlap_mm <= EPS);
                if pieces.is_empty() {
                    continue;
                }
                let local = transforms[r_index];
                for piece in pieces {
                    if piece.len() < 2 {
                        continue;
                    }
                    // 不再逐段直写：先并入该区域的合并链，断链时把前一条写出。
                    let flushed = pending[r_index].push(&piece);
                    write_chain(flushed, &mut outs[r_index], local, unit)?;
                }
            }
        } else if starts_with_ascii_ci_bytes(token_bytes, b"$$HATCHES/")
            || starts_with_ascii_ci_bytes(token_bytes, b"$$HATCH/")
        {
            first_geometry_seen = true;
            if outputs.is_none() {
                outputs = Some(open_outputs(
                    output_root,
                    stem,
                    index,
                    naming_template,
                    &header,
                    &preamble,
                    transforms,
                    regions,
                    unit,
                    write_capacity,
                )?);
            }
            let outs = outputs.as_mut().expect("outputs opened");
            // 与参考实现一致：HATCHES 打断 POLYLINE 链，先落盘所有未完成的链。
            for r_index in 0..REGION_COUNT {
                let flushed = pending[r_index].finish();
                write_chain(flushed, &mut outs[r_index], transforms[r_index], unit)?;
            }
            process_hatches_line_bytes(
                normalized_bytes,
                unit,
                outs,
                transforms,
                regions,
                overlap_mm,
                &mut hatch_scratch,
                intra_pool.as_ref(),
            )?;
        } else if !first_geometry_seen {
            // GEOMETRYSTART 后、首个几何实体前的少量元数据仍保留文本形式。
            preamble.push(String::from_utf8_lossy(normalized_bytes).into_owned());
        } else if starts_with_ascii_ci_bytes(token_bytes, b"$$LAYER/") {
            if let Some(outs) = outputs.as_mut() {
                // LAYER 同样打断 POLYLINE 链，先落盘再写层标记。
                for r_index in 0..REGION_COUNT {
                    let flushed = pending[r_index].finish();
                    write_chain(flushed, &mut outs[r_index], transforms[r_index], unit)?;
                }
                for out in outs {
                    write_raw_line(&mut out.writer, normalized_bytes)
                        .map_err(|e| format!("写入 LAYER 失败：{e}"))?;
                }
            }
        } else if let Some(outs) = outputs.as_mut() {
            // 未知几何扩展/GBK 注释按原始字节透传，不再做 lossy 转码。
            for out in outs {
                write_raw_line(&mut out.writer, normalized_bytes)
                    .map_err(|e| format!("透传未知 CLI 实体失败：{e}"))?;
            }
        }

        if last_emit.elapsed() >= PROGRESS_THROTTLE {
            let file_percent = if input_size > 0 {
                (line_end as f64 * 100.0 / input_size as f64).clamp(0.0, 99.9)
            } else {
                0.0
            };
            tracker.set_file_percent(slot, file_percent);
            emit_progress(
                app,
                tracker,
                total_files,
                slot,
                &file_name,
                "正在处理几何数据".to_string(),
            );
            last_emit = Instant::now();
        }
    }

    if !geometry_started || !geometry_finished {
        cleanup_temp_outputs(outputs);
        return Err(format!(
            "CLI 格式错误：{} 必须同时包含 $$GEOMETRYSTART 和 $$GEOMETRYEND",
            input_path.display()
        ));
    }

    // 几何阶段已结束：补一次该文件 100% 的进度，避免节流把最后一段吞掉导致进度停在 90% 多。
    tracker.set_file_percent(slot, 100.0);
    emit_progress(
        app,
        tracker,
        total_files,
        slot,
        &file_name,
        "正在写入分区结果".to_string(),
    );

    if outputs.is_none() {
        outputs = Some(open_outputs(
            output_root,
            stem,
            index,
            naming_template,
            &header,
            &preamble,
            transforms,
            regions,
            unit,
            write_capacity,
        )?);
    }

    // 几何流结束：落盘所有区域剩余的合并链，再收尾输出文件。
    {
        let outs = outputs.as_mut().expect("outputs exist");
        for r_index in 0..REGION_COUNT {
            let flushed = pending[r_index].finish();
            write_chain(flushed, &mut outs[r_index], transforms[r_index], unit)?;
        }
    }

    finalize_outputs(outputs.expect("outputs exist"))?;
    Ok(())
}

fn open_outputs(
    output_root: &Path,
    source_stem: &str,
    index: usize,
    naming_template: &str,
    header: &[String],
    preamble: &[String],
    transforms: &[Transform2D; REGION_COUNT],
    regions: &[Region; REGION_COUNT],
    unit: f64,
    write_capacity: usize,
) -> Result<Vec<OutputFile>, String> {
    let mut outputs = Vec::with_capacity(REGION_COUNT);
    for (r_index, base_region) in REGIONS.iter().copied().enumerate() {
        let region = regions[r_index];
        let file_name = render_name(naming_template, index, source_stem, base_region.name)?;
        let final_path = output_root
            .join(base_region.folder_id.to_string())
            .join(file_name);
        let temp_path = final_path.with_extension("cli.part");
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let file = File::create(&temp_path)
            .map_err(|e| format!("无法创建输出 {}：{e}", temp_path.display()))?;
        let mut writer = BufWriter::with_capacity(write_capacity, file);
        for line in header {
            if starts_with_ascii_ci(line.trim(), "$$LABEL/") {
                writeln!(writer, "$$LABEL/{}_{}", source_stem, base_region.name)
                    .map_err(|e| format!("写入 CLI 头失败：{e}"))?;
            } else if starts_with_ascii_ci(line.trim(), "$$DIMENSION/") {
                let rewritten = rewrite_split_dimension(line, region, transforms[r_index], unit)?;
                writeln!(writer, "{rewritten}")
                    .map_err(|e| format!("写入 CLI DIMENSION 失败：{e}"))?;
            } else {
                writeln!(writer, "{line}").map_err(|e| format!("写入 CLI 头失败：{e}"))?;
            }
        }
        writeln!(writer, "$$GEOMETRYSTART").map_err(|e| format!("写入 CLI 失败：{e}"))?;
        for line in preamble {
            writeln!(writer, "{line}").map_err(|e| format!("写入 CLI 失败：{e}"))?;
        }
        outputs.push(OutputFile {
            final_path,
            temp_path,
            writer,
            next_id: 1,
            line_buffer: Vec::with_capacity(64 * 1024),
        });
    }
    Ok(outputs)
}

fn rewrite_split_dimension(
    original: &str,
    region: Region,
    transform: Transform2D,
    unit: f64,
) -> Result<String, String> {
    let payload = original
        .split_once('/')
        .map(|(_, rhs)| rhs)
        .ok_or_else(|| format!("DIMENSION 格式错误：{original}"))?;
    let values: Vec<f64> = payload
        .split(',')
        .map(|v| parse_f64(v, "DIMENSION"))
        .collect::<Result<_, _>>()?;
    if values.len() != 6 {
        return Err(format!("DIMENSION 必须包含 6 个数值：{original}"));
    }
    let corners = [
        transform.apply((region.xmin, region.ymin)),
        transform.apply((region.xmin, region.ymax)),
        transform.apply((region.xmax, region.ymin)),
        transform.apply((region.xmax, region.ymax)),
    ];
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for (x, y) in corners {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    let inv = 1.0 / unit;
    Ok(format!(
        "$$DIMENSION/{},{},{},{},{},{}",
        format_cli_number(xmin * inv),
        format_cli_number(ymin * inv),
        format_cli_number(values[2]),
        format_cli_number(xmax * inv),
        format_cli_number(ymax * inv),
        format_cli_number(values[5]),
    ))
}

fn format_cli_number(mut value: f64) -> String {
    if value.abs() < 5e-10 {
        value = 0.0;
    }
    value = (value * 1_000_000.0).round() / 1_000_000.0;
    if value == -0.0 {
        value = 0.0;
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    text.strip_suffix(".0").unwrap_or(text).to_string()
}

fn finalize_outputs(mut outputs: Vec<OutputFile>) -> Result<(), String> {
    for out in &mut outputs {
        writeln!(out.writer, "$$GEOMETRYEND").map_err(|e| format!("写入 CLI 结束标记失败：{e}"))?;
        out.writer
            .flush()
            .map_err(|e| format!("刷新输出失败：{e}"))?;
    }

    let mut paths = Vec::with_capacity(outputs.len());
    for out in outputs {
        let OutputFile {
            final_path,
            temp_path,
            writer,
            ..
        } = out;
        drop(writer);
        let backup_path = final_path.with_extension("cli.replace.bak");
        paths.push((final_path, temp_path, backup_path));
    }

    // 先把旧正式文件改名为备份，确保 24 路提交失败时可以回滚。
    let mut backed_up = 0usize;
    for (final_path, _, backup_path) in &paths {
        if backup_path.exists() {
            let _ = fs::remove_file(backup_path);
        }
        if final_path.exists() {
            if let Err(e) = fs::rename(final_path, backup_path) {
                for (old_final, _, old_backup) in &paths[..backed_up] {
                    if old_backup.exists() {
                        let _ = fs::rename(old_backup, old_final);
                    }
                }
                for (_, temp, _) in &paths {
                    let _ = fs::remove_file(temp);
                }
                return Err(format!(
                    "无法准备覆盖已有输出 {}：{e}",
                    final_path.display()
                ));
            }
        }
        backed_up += 1;
    }

    let mut committed = 0usize;
    for (final_path, temp_path, _) in &paths {
        if let Err(e) = fs::rename(temp_path, final_path) {
            // 删除本轮已经提交的新文件，并恢复旧文件备份。
            for (new_final, _, _) in &paths[..committed] {
                let _ = fs::remove_file(new_final);
            }
            for (old_final, _, backup_path) in &paths {
                if backup_path.exists() {
                    let _ = fs::rename(backup_path, old_final);
                }
            }
            for (_, pending_temp, _) in &paths[committed..] {
                let _ = fs::remove_file(pending_temp);
            }
            return Err(format!("完成输出文件 {} 失败：{e}", final_path.display()));
        }
        committed += 1;
    }

    for (_, _, backup_path) in &paths {
        if backup_path.exists() {
            let _ = fs::remove_file(backup_path);
        }
    }
    Ok(())
}

fn cleanup_temp_outputs(outputs: Option<Vec<OutputFile>>) {
    if let Some(outputs) = outputs {
        for out in outputs {
            let path = out.temp_path.clone();
            drop(out.writer);
            let _ = fs::remove_file(path);
        }
    }
}

fn validate_request(request: &SplitRequest) -> Result<(), String> {
    if request.input_files.is_empty() {
        return Err("请先选择至少一个 CLI 文件".to_string());
    }
    if request.output_dir.trim().is_empty() {
        return Err("请选择输出文件夹".to_string());
    }
    let overlap_mm = request.overlap_mm.unwrap_or(2.0);
    if !overlap_mm.is_finite() || !(0.0..=20.0).contains(&overlap_mm) {
        return Err(format!(
            "重叠区域必须在 0～20 mm 之间，当前值：{overlap_mm}"
        ));
    }
    if request.transforms.len() < REGION_COUNT {
        return Err(format!(
            "分割参数不完整：需要 24 个振镜变换，当前只有 {} 个",
            request.transforms.len()
        ));
    }
    let mut seen = HashSet::new();
    for input in &request.input_files {
        let path = Path::new(input);
        let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("cli") {
            return Err(format!("仅支持 .cli 输入文件：{input}"));
        }
        let absolute = absolute_for_compare(path);
        if !seen.insert(absolute.clone()) {
            return Err(format!("输入文件重复：{}", absolute.display()));
        }
    }
    Ok(())
}

fn validate_output_collisions(request: &SplitRequest, output_root: &Path) -> Result<(), String> {
    let inputs: HashSet<String> = request
        .input_files
        .iter()
        .map(|p| path_compare_key(Path::new(p)))
        .collect();
    let mut outputs: HashSet<String> =
        HashSet::with_capacity(request.input_files.len() * REGION_COUNT);

    for (file_index, input) in request.input_files.iter().enumerate() {
        let input_path = Path::new(input);
        let stem = input_path
            .file_stem()
            .and_then(|v| v.to_str())
            .ok_or_else(|| format!("无法读取文件名：{}", input_path.display()))?;
        for region in REGIONS {
            let name = render_name(&request.naming_template, file_index + 1, stem, region.name)?;
            let final_path = output_root.join(region.folder_id.to_string()).join(name);
            let key = path_compare_key(&final_path);
            if inputs.contains(&key) {
                return Err(format!(
                    "分割输出不能覆盖输入文件：{}",
                    final_path.display()
                ));
            }
            if !outputs.insert(key) {
                return Err(format!(
                    "分割输出文件名发生冲突：{}。请在命名规则中加入 {{index}} 或 {{source_stem}} 以确保每个输入文件输出唯一。",
                    final_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn path_compare_key(path: &Path) -> String {
    let normalized = absolute_for_compare(path)
        .to_string_lossy()
        .replace('\\', "/");
    #[cfg(windows)]
    {
        normalized.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

fn cleanup_expected_temp_paths(output_root: &Path, stem: &str, index: usize, template: &str) {
    for region in REGIONS {
        if let Ok(name) = render_name(template, index, stem, region.name) {
            let final_path = output_root.join(region.folder_id.to_string()).join(name);
            let temp_path = final_path.with_extension("cli.part");
            let _ = fs::remove_file(temp_path);
        }
    }
}

fn absolute_for_compare(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

fn build_transform_table(items: &[MirrorTransform]) -> Result<[Transform2D; REGION_COUNT], String> {
    let mut table = [Transform2D {
        a: 1.0,
        b: 0.0,
        tx: 0.0,
        ty: 0.0,
    }; REGION_COUNT];
    let mut found = [false; REGION_COUNT];

    for item in items {
        let source = item.source.trim();
        if !source.eq_ignore_ascii_case("C3") {
            continue;
        }
        if let Some(index) = REGIONS
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(item.target.trim()))
        {
            table[index] = Transform2D {
                a: item.a,
                b: item.b,
                tx: item.tx,
                ty: item.ty,
            };
            found[index] = true;
        }
    }

    let missing: Vec<&str> = REGIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| !found[*i])
        .map(|(_, r)| r.name)
        .collect();
    if !missing.is_empty() {
        return Err(format!("参数缺少振镜：{}", missing.join(", ")));
    }
    Ok(table)
}

fn mb_capacity(value: usize, default_mb: usize) -> usize {
    let mb = if value == 0 {
        default_mb
    } else {
        value.clamp(1, 512)
    };
    mb * 1024 * 1024
}

fn available_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1)
}

fn largest_input_file_size(input_files: &[String]) -> u64 {
    input_files
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .max()
        .unwrap_or(0)
}

/// v14.1 文件级并发策略：
///
/// 1. 自动模式按文件数分档递增，上限 6；
/// 2. 大文件自动降并发（≥16 GB cap 4，≥32 GB cap 3）；
/// 3. 手动 worker 也设置硬上限；
/// 4. worker 永远不超过输入文件数。
fn determine_file_workers(input_files: &[String], requested_workers: usize) -> usize {
    let total = input_files.len();

    if total <= 1 {
        return 1;
    }

    let cpu_count = available_cpu_count();
    let largest_file = largest_input_file_size(input_files);

    let base_workers = if requested_workers == 0 {
        match total {
            0 | 1 => 1,
            2 => 2,
            3..=4 => 3,
            5..=8 => 4,
            _ => AUTO_MAX_FILE_WORKERS,
        }
    } else {
        requested_workers.min(MANUAL_MAX_FILE_WORKERS)
    };

    let size_cap = file_worker_size_cap(largest_file);

    base_workers.min(size_cap).min(cpu_count).min(total).max(1)
}

fn file_worker_size_cap(largest_file: u64) -> usize {
    if largest_file >= HUGE_FILE_THRESHOLD {
        3
    } else if largest_file >= LARGE_FILE_THRESHOLD {
        4
    } else {
        AUTO_MAX_FILE_WORKERS
    }
}

/// 单 CLI 内部几何并行线程数。
/// 只用于 total == 1，绝不能和文件级并行同时启用。
fn determine_geometry_workers(requested_workers: usize) -> usize {
    let cpu_count = available_cpu_count();

    if requested_workers == 0 {
        cpu_count.min(AUTO_MAX_GEOMETRY_WORKERS).max(1)
    } else {
        requested_workers
            .min(MANUAL_MAX_GEOMETRY_WORKERS)
            .min(cpu_count)
            .max(1)
    }
}

/// 全任务进度聚合器（按文件字节大小加权）。
///
/// 并行模式下各 worker 处理的文件顺序与输入序号无关，若只报自己那一个文件的进度，
/// N 个文件同时跑到 50% 时每个 worker 都只会报 0.5/N 的整体进度，前端会严重落后于真实进度。
/// 因此这里按槽位记录每个文件的当前百分比，上报时把所有文件的贡献累加。
///
/// 权重：每个文件按其字节数占总字节数的比例贡献进度。
/// 等权重公式（完成数+在途百分比）/文件数在文件大小差异大时会失真：
/// 若干小文件几秒跑完就能把进度条推到 90%+，随后的大文件却只占 1/N 权重，
/// 造成"进度条虚高后长时间停滞"。字节加权后进度条与真实工作量线性对应。
/// 所有文件大小均未知（全为 0）时退化为等权重。
struct ProgressTracker {
    completed: AtomicUsize,
    /// 每个文件的当前百分比定点值：0..10_000 表示 0%..100%
    per_file: Vec<AtomicU32>,
    /// 每个文件的进度权重（字节数；未知时为 1）
    weights: Vec<f64>,
    total_weight: f64,
}

impl ProgressTracker {
    fn new(file_sizes: Vec<u64>) -> Self {
        let weights: Vec<f64> = if file_sizes.is_empty() {
            vec![1.0]
        } else {
            file_sizes.iter().map(|&s| s as f64).collect()
        };
        // 全部未知大小（全 0）时退化为等权重，避免整体进度恒为 0。
        let weights = if weights.iter().all(|&w| w <= 0.0) {
            vec![1.0; weights.len()]
        } else {
            weights
        };
        let total_weight = weights.iter().sum();
        Self {
            completed: AtomicUsize::new(0),
            per_file: (0..weights.len()).map(|_| AtomicU32::new(0)).collect(),
            weights,
            total_weight,
        }
    }

    fn set_file_percent(&self, file_index: usize, percent: f64) {
        let fixed = (percent.clamp(0.0, 100.0) * 100.0).round() as u32;
        self.per_file[file_index].store(fixed.min(10_000), AtomicOrdering::Release);
    }

    /// 标记一个文件处理完毕：槽位置 100%，完成数仅驱动"已处理"计数。
    /// 进度完全由槽位加权得出，因此不存在 completed 与槽位的重复计数。
    fn finish_file(&self, file_index: usize) -> usize {
        self.per_file[file_index].store(10_000, AtomicOrdering::Release);
        self.completed.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn own_percent(&self, file_index: usize) -> f64 {
        self.per_file[file_index].load(AtomicOrdering::Acquire) as f64 / 100.0
    }

    fn overall_percent(&self) -> f64 {
        let weighted: f64 = self
            .per_file
            .iter()
            .zip(&self.weights)
            .map(|(slot, weight)| slot.load(AtomicOrdering::Acquire) as f64 / 10_000.0 * weight)
            .sum();
        (weighted / self.total_weight * 100.0).clamp(0.0, 100.0)
    }

    fn advance(&self, file_index: usize) -> (usize, f64, f64) {
        let done = self.completed.load(AtomicOrdering::Acquire);
        (
            done,
            self.own_percent(file_index),
            self.overall_percent(),
        )
    }
}

fn emit_progress(
    app: &AppHandle,
    tracker: &ProgressTracker,
    total: usize,
    file_index: usize,
    current_file: &str,
    message: String,
) {
    let (completed, current_file_percent, overall) = tracker.advance(file_index);
    let _ = app.emit(
        EVENT_NAME,
        SplitProgress {
            percent: overall,
            completed,
            total,
            current_file: current_file.to_string(),
            current_file_percent,
            message,
        },
    );
}

fn parse_units(header: &[String]) -> Result<f64, String> {
    for line in header {
        let token = line.trim();
        if !starts_with_ascii_ci(token, "$$UNITS/") {
            continue;
        }
        let rhs = token
            .split_once('/')
            .map(|(_, rhs)| rhs.trim())
            .unwrap_or("");
        if rhs.is_empty() {
            return Ok(1.0);
        }
        let value = parse_f64(rhs, "UNITS")?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("UNITS 必须为正有限数值：{rhs}"));
        }
        return Ok(value);
    }
    Ok(1.0)
}

fn parse_polyline_bytes(line: &[u8], unit: f64) -> Result<Polyline, String> {
    let slash = line
        .iter()
        .position(|&b| b == b'/')
        .ok_or_else(|| "POLYLINE 格式错误：缺少 /".to_string())?;
    let payload = &line[slash + 1..];
    let mut it = payload.split(|&b| b == b',');
    let _id = next_field_bytes(&mut it, "POLYLINE ID")?;
    // kind 仍参与格式校验；合并输出统一写 kind=0（与参考实现一致），故不再保留。
    let _kind = parse_i32_bytes(next_field_bytes(&mut it, "POLYLINE 类型")?, "POLYLINE 类型")?;
    let count = parse_usize_bytes(next_field_bytes(&mut it, "POLYLINE 点数")?, "POLYLINE 点数")?;
    if count < 2 || count > 1_000_000 {
        return Err(format!("POLYLINE 点数异常：{count}"));
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        let x = parse_f64_bytes(next_field_bytes(&mut it, "POLYLINE X")?, "POLYLINE X")? * unit;
        let y = parse_f64_bytes(next_field_bytes(&mut it, "POLYLINE Y")?, "POLYLINE Y")? * unit;
        points.push((x, y));
    }
    Ok(Polyline { points })
}

fn normalize_cli_line_bytes(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b'\r' | b'\n') {
        end -= 1;
    }
    let mut out = &line[..end];
    if out.starts_with(&[0xEF, 0xBB, 0xBF]) {
        out = &out[3..];
    }
    out
}

#[inline]
fn trim_ascii_bytes(mut value: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = value.split_first() {
        if first.is_ascii_whitespace() {
            value = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = value.split_last() {
        if last.is_ascii_whitespace() {
            value = rest;
        } else {
            break;
        }
    }
    value
}

#[inline]
fn eq_ascii_ci_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| x.eq_ignore_ascii_case(&y))
}

#[inline]
fn starts_with_ascii_ci_bytes(value: &[u8], prefix: &[u8]) -> bool {
    value.len() >= prefix.len()
        && value[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(&x, &y)| x.eq_ignore_ascii_case(&y))
}

#[inline]
fn next_field_bytes<'a>(
    it: &mut impl Iterator<Item = &'a [u8]>,
    name: &str,
) -> Result<&'a [u8], String> {
    it.next()
        .map(trim_ascii_bytes)
        .ok_or_else(|| format!("缺少字段：{name}"))
}

#[inline]
fn parse_i32_bytes(value: &[u8], name: &str) -> Result<i32, String> {
    lexical_core::parse::<i32>(trim_ascii_bytes(value))
        .map_err(|_| format!("{name} 不是整数：{}", String::from_utf8_lossy(value)))
}

#[inline]
fn parse_usize_bytes(value: &[u8], name: &str) -> Result<usize, String> {
    lexical_core::parse::<usize>(trim_ascii_bytes(value))
        .map_err(|_| format!("{name} 不是整数：{}", String::from_utf8_lossy(value)))
}

#[inline]
fn parse_f64_bytes(value: &[u8], name: &str) -> Result<f64, String> {
    lexical_core::parse::<f64>(trim_ascii_bytes(value))
        .map_err(|_| format!("{name} 不是数值：{}", String::from_utf8_lossy(value)))
}

#[inline]
fn write_raw_line(writer: &mut BufWriter<File>, line: &[u8]) -> std::io::Result<()> {
    writer.write_all(line)?;
    writer.write_all(b"\n")
}

fn parse_f64(value: &str, name: &str) -> Result<f64, String> {
    lexical_core::parse::<f64>(value.trim().as_bytes())
        .map_err(|_| format!("{name} 不是数值：{value}"))
}

fn starts_with_ascii_ci(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .map(|head| head.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn region_with_overlap(index: usize, overlap_mm: f64) -> Region {
    let mut region = REGIONS[index];
    let half = (overlap_mm.max(0.0)) * 0.5;
    if half <= EPS {
        return region;
    }
    if region.xmin > X_MIN + EPS {
        region.xmin -= half;
    }
    if region.xmax < FIELD_X_MAX - EPS {
        region.xmax += half;
    }
    if region.ymin > FIELD_Y_MIN + EPS {
        region.ymin -= half;
    }
    if region.ymax < FIELD_Y_MAX - EPS {
        region.ymax += half;
    }
    region
}

fn build_effective_regions(overlap_mm: f64) -> [Region; REGION_COUNT] {
    if overlap_mm <= EPS {
        // Region 是 Copy，因此零重叠时直接复制静态表，无需逐项计算。
        return REGIONS;
    }
    std::array::from_fn(|index| region_with_overlap(index, overlap_mm))
}

#[inline(always)]
fn bbox_intersects_region(b: (f64, f64, f64, f64), r: Region) -> bool {
    !(b.1 < r.xmin - EPS || b.0 > r.xmax + EPS || b.3 < r.ymin - EPS || b.2 > r.ymax + EPS)
}

#[inline]
fn candidate_regions_for_bbox(
    b: (f64, f64, f64, f64),
    regions: &[Region; REGION_COUNT],
    overlap_mm: f64,
) -> ([usize; REGION_COUNT], usize) {
    let mut out = [0usize; REGION_COUNT];
    let mut count = 0usize;
    let half = overlap_mm.max(0.0) * 0.5;

    // overlap 只扩大内部边界，因此候选列也按半重叠量向两侧放宽。
    let raw_start = (((b.0 - half) - X_MIN) / CELL_W).floor() as isize - 1;
    let raw_end = (((b.1 + half) - X_MIN) / CELL_W).floor() as isize + 1;
    let start = raw_start.clamp(0, COLS as isize - 1) as usize;
    let end = raw_end.clamp(0, COLS as isize - 1) as usize;

    for col in start..=end {
        for r_index in [TOP_BY_COL[col], BOTTOM_BY_COL[col]] {
            let region = regions[r_index];
            if bbox_intersects_region(b, region) {
                out[count] = r_index;
                count += 1;
            }
        }
    }
    (out, count)
}

fn process_hatches_line_bytes(
    line: &[u8],
    unit: f64,
    outs: &mut [OutputFile],
    transforms: &[Transform2D; REGION_COUNT],
    regions: &[Region; REGION_COUNT],
    overlap_mm: f64,
    scratch: &mut [Vec<HatchSegment>],
    intra_pool: Option<&rayon::ThreadPool>,
) -> Result<(), String> {
    for bucket in scratch.iter_mut() {
        bucket.clear();
    }

    let slash = line
        .iter()
        .position(|&b| b == b'/')
        .ok_or_else(|| "HATCHES 格式错误：缺少 /".to_string())?;
    let payload = &line[slash + 1..];

    // 两种允许格式：id,count,(x1,y1,x2,y2)*N 或 id,kind,count,(...)*N。
    // 全程使用 &[u8] 字段切片，不构造 UTF-8 String。
    let field_count = payload.iter().filter(|&&b| b == b',').count() + 1;
    let mut probe = payload.split(|&b| b == b',');
    let _id_probe = next_field_bytes(&mut probe, "HATCHES ID")?;
    let second_text = next_field_bytes(&mut probe, "HATCHES 数量/类型")?;
    let second_as_count = parse_usize_bytes(second_text, "HATCHES 数量/类型").ok();
    let third_text = probe.next().map(trim_ascii_bytes);
    let third_as_count = third_text.and_then(|v| parse_usize_bytes(v, "HATCHES 线段数").ok());

    let format1 = second_as_count
        .map(|n| field_count == 2usize.saturating_add(n.saturating_mul(4)))
        .unwrap_or(false);
    let format2 = third_as_count
        .map(|n| field_count == 3usize.saturating_add(n.saturating_mul(4)))
        .unwrap_or(false);

    if format1 == format2 {
        return Err(format!("HATCHES 格式无法识别：字段数={field_count}"));
    }

    let mut it = payload.split(|&b| b == b',');
    let _id = next_field_bytes(&mut it, "HATCHES ID")?;
    let second_text = next_field_bytes(&mut it, "HATCHES 数量/类型")?;
    let (kind, count) = if format1 {
        (0, parse_usize_bytes(second_text, "HATCHES 线段数")?)
    } else {
        let kind = parse_i32_bytes(second_text, "HATCHES 类型")?;
        let count = parse_usize_bytes(
            next_field_bytes(&mut it, "HATCHES 线段数")?,
            "HATCHES 线段数",
        )?;
        (kind, count)
    };

    if count > 1_000_000 {
        return Err(format!("HATCHES 线段数异常：{count}"));
    }

    // 小实体串行更快；中/大实体根据实际线程数自适应启用并行。
    // 固定 4096 的旧阈值会让“很多 1k~3k 线段 HATCHES”的文件长期只跑单核。
    let parallel_threshold = intra_pool
        .map(|pool| pool.current_num_threads().saturating_mul(256).max(512))
        .unwrap_or(usize::MAX);

    if let Some(pool) = intra_pool.filter(|_| count >= parallel_threshold) {
        let threads = pool.current_num_threads().max(1);
        let parallel_chunk = (count / threads.saturating_mul(4).max(1)).clamp(128, 2048);
        let mut segments = Vec::with_capacity(count);
        for _ in 0..count {
            let x1 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES x1")?, "HATCHES x1")? * unit;
            let y1 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES y1")?, "HATCHES y1")? * unit;
            let x2 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES x2")?, "HATCHES x2")? * unit;
            let y2 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES y2")?, "HATCHES y2")? * unit;
            segments.push((x1, y1, x2, y2));
        }

        let chunk_results: Vec<[Vec<HatchSegment>; REGION_COUNT]> = pool.install(|| {
            segments
                .par_chunks(parallel_chunk)
                .map(|chunk| {
                    let mut local_buckets: [Vec<HatchSegment>; REGION_COUNT] =
                        std::array::from_fn(|_| Vec::new());
                    for &(x1, y1, x2, y2) in chunk {
                        let bbox = (x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2));
                        let (candidates, candidate_count) =
                            candidate_regions_for_bbox(bbox, regions, overlap_mm);
                        for &r_index in &candidates[..candidate_count] {
                            let region = regions[r_index];
                            if let Some((cx1, cy1, cx2, cy2)) =
                                clip_segment(x1, y1, x2, y2, region, overlap_mm <= EPS)
                            {
                                let local = transforms[r_index];
                                let p1 = local.apply((cx1, cy1));
                                let p2 = local.apply((cx2, cy2));
                                local_buckets[r_index].push((p1.0, p1.1, p2.0, p2.1));
                            }
                        }
                    }
                    local_buckets
                })
                .collect()
        });

        for buckets in chunk_results {
            for r_index in 0..REGION_COUNT {
                scratch[r_index].extend_from_slice(&buckets[r_index]);
            }
        }
    } else {
        for _ in 0..count {
            let x1 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES x1")?, "HATCHES x1")? * unit;
            let y1 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES y1")?, "HATCHES y1")? * unit;
            let x2 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES x2")?, "HATCHES x2")? * unit;
            let y2 =
                parse_f64_bytes(next_field_bytes(&mut it, "HATCHES y2")?, "HATCHES y2")? * unit;

            let bbox = (x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2));
            let (candidates, candidate_count) =
                candidate_regions_for_bbox(bbox, regions, overlap_mm);
            for &r_index in &candidates[..candidate_count] {
                let region = regions[r_index];
                if let Some((cx1, cy1, cx2, cy2)) =
                    clip_segment(x1, y1, x2, y2, region, overlap_mm <= EPS)
                {
                    let local = transforms[r_index];
                    let p1 = local.apply((cx1, cy1));
                    let p2 = local.apply((cx2, cy2));
                    scratch[r_index].push((p1.0, p1.1, p2.0, p2.1));
                }
            }
        }
    }

    for r_index in 0..REGION_COUNT {
        let clipped = &scratch[r_index];
        if clipped.is_empty() {
            continue;
        }
        let id = outs[r_index].next_id;
        outs[r_index].next_id += 1;
        write_hatches_buffered(&mut outs[r_index], id, kind, clipped, unit)?;
    }
    Ok(())
}

fn points_bbox(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for &(x, y) in points {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    (xmin, xmax, ymin, ymax)
}

#[cfg(test)]
fn bbox_intersects(b: (f64, f64, f64, f64), r: Region) -> bool {
    bbox_intersects_region(b, r)
}

fn clip_polyline(
    points: &[(f64, f64)],
    region: Region,
    unique_shared_boundary: bool,
) -> Vec<Vec<(f64, f64)>> {
    let mut pieces: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    for pair in points.windows(2) {
        let (x1, y1) = pair[0];
        let (x2, y2) = pair[1];
        if let Some((cx1, cy1, cx2, cy2)) =
            clip_segment(x1, y1, x2, y2, region, unique_shared_boundary)
        {
            let p1 = (cx1, cy1);
            let p2 = (cx2, cy2);
            if current.is_empty() {
                current.push(p1);
                current.push(p2);
            } else if same_point(*current.last().unwrap(), p1) {
                if !same_point(*current.last().unwrap(), p2) {
                    current.push(p2);
                }
            } else {
                if current.len() >= 2 {
                    pieces.push(std::mem::take(&mut current));
                }
                current.push(p1);
                current.push(p2);
            }
        } else if current.len() >= 2 {
            pieces.push(std::mem::take(&mut current));
        }
    }
    if current.len() >= 2 {
        pieces.push(current);
    }
    pieces
}

fn same_point(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() <= 1e-6 && (a.1 - b.1).abs() <= 1e-6
}

#[inline]
fn clip_segment(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    r: Region,
    unique_shared_boundary: bool,
) -> Option<(f64, f64, f64, f64)> {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let mut u1: f64 = 0.0;
    let mut u2: f64 = 1.0;
    let p = [-dx, dx, -dy, dy];
    let q = [x1 - r.xmin, r.xmax - x1, y1 - r.ymin, r.ymax - y1];
    for i in 0..4 {
        if p[i].abs() < EPS {
            if q[i] < -EPS {
                return None;
            }
            continue;
        }
        let t = q[i] / p[i];
        if p[i] < 0.0 {
            if t > u2 + EPS {
                return None;
            }
            u1 = u1.max(t);
        } else {
            if t < u1 - EPS {
                return None;
            }
            u2 = u2.min(t);
        }
    }
    if u2 < u1 - EPS {
        return None;
    }
    let clipped = (x1 + u1 * dx, y1 + u1 * dy, x1 + u2 * dx, y1 + u2 * dy);
    if unique_shared_boundary && !owns_shared_boundary_segment(r, clipped) {
        return None;
    }
    Some(clipped)
}

// 接缝采用半开区间所有权：共享竖边归右侧区域，共享横边归上侧区域。
// 穿越接缝的线段仍会在两边各保留自己的片段；只有“整段恰好压在接缝上”时去重。
fn owns_shared_boundary_segment(r: Region, s: (f64, f64, f64, f64)) -> bool {
    let on_right = (s.0 - r.xmax).abs() <= EPS && (s.2 - r.xmax).abs() <= EPS;
    if on_right && r.xmax < FIELD_X_MAX - EPS {
        return false;
    }
    let on_top = (s.1 - r.ymax).abs() <= EPS && (s.3 - r.ymax).abs() <= EPS;
    if on_top && r.ymax < FIELD_Y_MAX - EPS {
        return false;
    }
    true
}

fn write_polyline_buffered(
    out: &mut OutputFile,
    id: u64,
    kind: i32,
    c3_points: &[(f64, f64)],
    local: Transform2D,
    unit: f64,
) -> Result<(), String> {
    let inv = 1.0 / unit;
    let buf = &mut out.line_buffer;
    buf.clear();
    buf.extend_from_slice(b"$$POLYLINE/");
    append_u64(buf, id);
    buf.push(b',');
    append_i32(buf, kind);
    buf.push(b',');
    append_usize(buf, c3_points.len());
    for &p in c3_points {
        let q = local.apply(p);
        buf.push(b',');
        append_cli_number(buf, q.0 * inv);
        buf.push(b',');
        append_cli_number(buf, q.1 * inv);
    }
    buf.push(b'\n');
    out.writer
        .write_all(buf)
        .map_err(|e| format!("写入 POLYLINE 失败：{e}"))
}

fn write_hatches_buffered(
    out: &mut OutputFile,
    id: u64,
    kind: i32,
    local_segments: &[(f64, f64, f64, f64)],
    unit: f64,
) -> Result<(), String> {
    let inv = 1.0 / unit;
    let buf = &mut out.line_buffer;
    buf.clear();
    // 按约 18~24 bytes/坐标预留，减少超长 HATCHES 行扩容次数；容量只增长不收缩，后续复用。
    let expected = 32usize.saturating_add(local_segments.len().saturating_mul(80));
    if buf.capacity() < expected {
        buf.reserve(expected - buf.capacity());
    }
    buf.extend_from_slice(b"$$HATCHES/");
    append_u64(buf, id);
    buf.push(b',');
    if kind != 0 {
        append_i32(buf, kind);
        buf.push(b',');
    }
    append_usize(buf, local_segments.len());
    for &(x1, y1, x2, y2) in local_segments {
        for value in [x1 * inv, y1 * inv, x2 * inv, y2 * inv] {
            buf.push(b',');
            append_cli_number(buf, value);
        }
    }
    buf.push(b'\n');
    out.writer
        .write_all(buf)
        .map_err(|e| format!("写入 HATCHES 失败：{e}"))
}

#[inline]
fn append_cli_number(buf: &mut Vec<u8>, mut value: f64) {
    if value.abs() < 5e-10 {
        value = 0.0;
    }
    value = (value * 1_000_000.0).round() / 1_000_000.0;
    if value == -0.0 {
        value = 0.0;
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    let text = text.strip_suffix(".0").unwrap_or(text);
    buf.extend_from_slice(text.as_bytes());
}

#[inline]
fn append_u64(buf: &mut Vec<u8>, value: u64) {
    let mut tmp = itoa::Buffer::new();
    buf.extend_from_slice(tmp.format(value).as_bytes());
}

#[inline]
fn append_usize(buf: &mut Vec<u8>, value: usize) {
    let mut tmp = itoa::Buffer::new();
    buf.extend_from_slice(tmp.format(value).as_bytes());
}

#[inline]
fn append_i32(buf: &mut Vec<u8>, value: i32) {
    let mut tmp = itoa::Buffer::new();
    buf.extend_from_slice(tmp.format(value).as_bytes());
}

fn render_name(
    template: &str,
    index: usize,
    source_stem: &str,
    mirror: &str,
) -> Result<String, String> {
    let mut name = template
        .replace("{index:03}", &format!("{index:03}"))
        .replace("{index}", &index.to_string())
        .replace("{source_stem}", source_stem)
        .replace("{mirror}", mirror);
    if !name.to_ascii_lowercase().ends_with(".cli") {
        name.push_str(".cli");
    }
    if name.contains('/') || name.contains('\\') || name == ".cli" {
        return Err(format!("分割文件命名规则生成了无效文件名：{name}"));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyline_chain_merges_connected_pieces() {
        // 两段共点线段应合并成一条 3 点链，只落盘一次。
        let mut chain = PolylineChain::new();
        assert_eq!(chain.push(&[(0.0, 0.0), (1.0, 0.0)]), None);
        assert_eq!(chain.push(&[(1.0, 0.0), (2.0, 5.0)]), None);
        let finished = chain.finish().expect("应得到一条合并链");
        assert_eq!(
            finished,
            vec![(0.0, 0.0), (1.0, 0.0), (2.0, 5.0)],
            "共享点 (1,0) 只保留一份"
        );
    }

    #[test]
    fn polyline_chain_breaks_on_discontinuity() {
        // 端点不连续时，push 返回前一条链，新片段重新开链。
        let mut chain = PolylineChain::new();
        assert_eq!(chain.push(&[(0.0, 0.0), (1.0, 0.0)]), None);
        let flushed = chain.push(&[(5.0, 5.0), (6.0, 6.0)]).expect("应断链");
        assert_eq!(flushed, vec![(0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(chain.finish(), Some(vec![(5.0, 5.0), (6.0, 6.0)]));
    }

    #[test]
    fn polyline_chain_tolerates_epsilon_gap() {
        // 1e-6 mm 容差内的间隙视为连续（与参考实现 same_point 一致）。
        let mut chain = PolylineChain::new();
        assert_eq!(chain.push(&[(0.0, 0.0), (1.0, 0.0)]), None);
        assert_eq!(chain.push(&[(1.0 + 5e-7, 0.0), (2.0, 0.0)]), None);
        assert_eq!(
            chain.finish(),
            Some(vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)])
        );
    }

    #[test]
    fn polyline_chain_finish_without_enough_points_is_none() {
        let mut chain = PolylineChain::new();
        assert_eq!(chain.finish(), None, "空链 finish 应为 None");
        // push 后 finish 消费掉内容，再次 finish 为 None。
        let mut chain = PolylineChain::new();
        assert_eq!(chain.push(&[(0.0, 0.0), (1.0, 1.0)]), None);
        assert!(chain.finish().is_some());
        assert_eq!(chain.finish(), None);
    }

    #[test]
    fn mapping_is_complete_and_unique() {
        let mut ids = REGIONS.iter().map(|r| r.folder_id).collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, (1u8..=24).collect::<Vec<_>>());
    }

    #[test]
    fn naming_template_works() {
        assert_eq!(
            render_name("{index:03}_{source_stem}_{mirror}.cli", 2, "model", "C3").unwrap(),
            "002_model_C3.cli"
        );
    }

    #[test]
    fn clipping_works() {
        let r = Region {
            name: "X",
            xmin: 0.0,
            xmax: 10.0,
            ymin: 0.0,
            ymax: 10.0,
            folder_id: 1,
        };
        let c = clip_segment(-5.0, 5.0, 15.0, 5.0, r, true).unwrap();
        assert!((c.0 - 0.0).abs() < 1e-9);
        assert!((c.2 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn seam_line_has_single_owner() {
        let top = REGIONS[0];
        let bottom = REGIONS[1];
        let top_hit = clip_segment(-1400.0, -140.0, -1200.0, -140.0, top, true).is_some();
        let bottom_hit = clip_segment(-1400.0, -140.0, -1200.0, -140.0, bottom, true).is_some();
        assert!(top_hit);
        assert!(!bottom_hit);
    }

    #[test]
    fn two_mm_overlap_creates_two_mm_shared_band() {
        let regions = build_effective_regions(2.0);
        let top = regions[0];
        let bottom = regions[1];
        assert!((top.ymin + 141.0).abs() < 1e-9);
        assert!((bottom.ymax + 139.0).abs() < 1e-9);
        assert!((bottom.ymax - top.ymin - 2.0).abs() < 1e-9);

        // 位于原接缝上的线在 overlap>0 时应同时进入上下两个振镜。
        assert!(clip_segment(-1400.0, -140.0, -1200.0, -140.0, top, false).is_some());
        assert!(clip_segment(-1400.0, -140.0, -1200.0, -140.0, bottom, false).is_some());
    }

    #[test]
    fn candidate_index_matches_bruteforce() {
        for xi in -16..20 {
            for yi in -6..5 {
                let x0 = xi as f64 * 100.0;
                let y0 = yi as f64 * 100.0;
                let bbox = (x0, x0 + 73.0, y0, y0 + 61.0);
                let regions = build_effective_regions(0.0);
                let (candidates, count) = candidate_regions_for_bbox(bbox, &regions, 0.0);
                let mut fast = candidates[..count].to_vec();
                fast.sort_unstable();
                let mut brute = REGIONS
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| bbox_intersects(bbox, **r))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                brute.sort_unstable();
                assert_eq!(fast, brute, "bbox={bbox:?}");
            }
        }
    }
    #[test]
    fn precomputed_regions_match_direct_overlap_calculation() {
        for overlap in [0.0, 0.1, 2.0, 7.5, 20.0] {
            let cached = build_effective_regions(overlap);
            for index in 0..REGION_COUNT {
                let direct = region_with_overlap(index, overlap);
                assert_eq!(cached[index].name, direct.name);
                assert!((cached[index].xmin - direct.xmin).abs() < 1e-12);
                assert!((cached[index].xmax - direct.xmax).abs() < 1e-12);
                assert!((cached[index].ymin - direct.ymin).abs() < 1e-12);
                assert!((cached[index].ymax - direct.ymax).abs() < 1e-12);
                assert_eq!(cached[index].folder_id, direct.folder_id);
            }
        }
    }

    #[test]
    fn output_name_collisions_are_rejected() {
        let request = SplitRequest {
            input_files: vec!["a.cli".into(), "b.cli".into()],
            output_dir: "out".into(),
            naming_template: "{mirror}.cli".into(),
            read_buffer_mb: None,
            write_buffer_mb: None,
            parallel: Some(true),
            worker_count: Some(2),
            overlap_mm: Some(2.0),
            transforms: vec![],
        };
        let output_root = std::env::temp_dir().join("cli-split-collision-test");
        let error = validate_output_collisions(&request, &output_root).unwrap_err();
        assert!(error.contains("冲突"));
    }

    #[test]
    fn invalid_units_are_rejected() {
        assert_eq!(parse_units(&["$$UNITS/1.0".into()]).unwrap(), 1.0);
        assert!(parse_units(&["$$UNITS/not-a-number".into()]).is_err());
        assert!(parse_units(&["$$UNITS/0".into()]).is_err());
    }

    #[test]
    fn byte_polyline_parser_matches_ascii_values() {
        let p = parse_polyline_bytes(b"$$POLYLINE/7,1,3,0,0,1.5,-2,3,4", 2.0).unwrap();
        assert_eq!(p.points.len(), 3);
        assert_eq!(p.points[0], (0.0, 0.0));
        assert_eq!(p.points[1], (3.0, -4.0));
        assert_eq!(p.points[2], (6.0, 8.0));
    }

    #[test]
    fn raw_line_normalization_keeps_non_utf8_bytes() {
        let raw = [
            0x24, 0x24, 0x4c, 0x41, 0x42, 0x45, 0x4c, 0x2f, 0x81, 0x82, b'\r', b'\n',
        ];
        let line = normalize_cli_line_bytes(&raw);
        assert_eq!(&line[line.len() - 2..], &[0x81, 0x82]);
    }

    #[test]
    fn parallel_progress_aggregates_all_files() {
        // 等权重退化场景（大小全为 0）：8 个文件并行各跑到 50%，
        // 整体应接近 50%，而不是某一个文件的 0.5/8 = 6.25%
        let tracker = ProgressTracker::new(vec![0; 8]);
        for slot in 0..8 {
            tracker.set_file_percent(slot, 50.0);
        }
        let overall = tracker.overall_percent();
        assert!(
            (overall - 50.0).abs() < 0.1,
            "整体进度应为 50%，实际 {overall}"
        );
        assert_eq!(tracker.advance(3).0, 0);
        assert!((tracker.own_percent(3) - 50.0).abs() < 0.1);
    }

    #[test]
    fn finished_files_are_not_double_counted() {
        // 大小未知 → 等权重退化
        let tracker = ProgressTracker::new(vec![0, 0]);
        tracker.set_file_percent(0, 80.0);
        tracker.set_file_percent(1, 40.0);
        assert!((tracker.overall_percent() - 60.0).abs() < 0.1);

        // 完成后槽位置 100%（而非清零），进度仍由槽位唯一决定，不会重复计数
        assert_eq!(tracker.finish_file(0), 1);
        assert!((tracker.own_percent(0) - 100.0).abs() < 0.1);
        assert!((tracker.overall_percent() - 70.0).abs() < 0.1);

        tracker.set_file_percent(1, 100.0);
        assert!((tracker.overall_percent() - 100.0).abs() < 0.1);
        assert_eq!(tracker.finish_file(1), 2);
        assert!((tracker.overall_percent() - 100.0).abs() < 0.1);
    }

    #[test]
    fn progress_is_weighted_by_file_size() {
        // 大小差异大时进度条应与字节工作量线性对应：
        // 1 KB 小文件全部完成只贡献 1/11 ≈ 9.1%，而不是等权重的 50%。
        let tracker = ProgressTracker::new(vec![1, 10]);
        tracker.set_file_percent(0, 100.0);
        tracker.finish_file(0);
        assert!((tracker.overall_percent() - 100.0 / 11.0).abs() < 0.1);

        // 大文件跑到 50% → (1 + 0.5*10)/11 ≈ 54.5%
        tracker.set_file_percent(1, 50.0);
        let expected = (1.0 + 5.0) / 11.0 * 100.0;
        assert!((tracker.overall_percent() - expected).abs() < 0.1);
    }

    #[test]
    fn single_file_progress_matches_file_percent() {
        let tracker = ProgressTracker::new(vec![123]);
        tracker.set_file_percent(0, 37.5);
        assert!((tracker.overall_percent() - 37.5).abs() < 0.1);
        tracker.set_file_percent(0, 100.0);
        assert!((tracker.overall_percent() - 100.0).abs() < 0.1);
    }

    #[test]
    fn v14_auto_file_workers_are_io_limited() {
        let files = vec![
            "__missing_1.cli".to_string(),
            "__missing_2.cli".to_string(),
            "__missing_3.cli".to_string(),
            "__missing_4.cli".to_string(),
            "__missing_5.cli".to_string(),
            "__missing_6.cli".to_string(),
        ];

        let workers = determine_file_workers(&files, 0);

        assert!(workers >= 1);
        assert!(workers <= AUTO_MAX_FILE_WORKERS);
        assert!(workers <= files.len());
    }

    #[test]
    fn v14_manual_file_workers_have_hard_cap() {
        let files = (0..20)
            .map(|i| format!("__missing_{i}.cli"))
            .collect::<Vec<_>>();

        let workers = determine_file_workers(&files, 64);

        assert!(workers <= MANUAL_MAX_FILE_WORKERS);
        assert!(workers <= AUTO_MAX_FILE_WORKERS);
    }

    #[test]
    fn v14_single_file_never_uses_file_parallelism() {
        let files = vec!["__missing.cli".to_string()];

        assert_eq!(determine_file_workers(&files, 8), 1);
    }

    #[test]
    fn v14_geometry_workers_are_capped() {
        let auto = determine_geometry_workers(0);
        let manual = determine_geometry_workers(64);

        assert!(auto >= 1);
        assert!(auto <= AUTO_MAX_GEOMETRY_WORKERS);

        assert!(manual >= 1);
        assert!(manual <= MANUAL_MAX_GEOMETRY_WORKERS);
    }

    #[test]
    fn v14_large_files_reduce_file_parallelism() {
        assert_eq!(file_worker_size_cap(1 * GB), 6);
        assert_eq!(file_worker_size_cap(15 * GB), 6);
        assert_eq!(file_worker_size_cap(16 * GB), 4);
        assert_eq!(file_worker_size_cap(31 * GB), 4);
        assert_eq!(file_worker_size_cap(32 * GB), 3);
        assert_eq!(file_worker_size_cap(64 * GB), 3);
    }

    #[test]
    fn v14_1_auto_file_workers_tier_by_count() {
        // 大小未知（0 字节）不触发降档，验证纯文件数分档：
        // 2→2、3..=4→3、5..=8→4、>8→6
        let make = |n: usize| (0..n).map(|i| format!("__missing_{i}.cli")).collect::<Vec<_>>();
        assert_eq!(determine_file_workers(&make(2), 0), 2);
        assert_eq!(determine_file_workers(&make(3), 0), 3);
        assert_eq!(determine_file_workers(&make(4), 0), 3);
        assert_eq!(determine_file_workers(&make(5), 0), 4);
        assert_eq!(determine_file_workers(&make(8), 0), 4);
        assert_eq!(determine_file_workers(&make(9), 0), 6);
        assert_eq!(determine_file_workers(&make(20), 0), 6);
    }

    #[test]
    fn v14_1_large_files_still_allow_parallelism() {
        // v14.1：大文件只降档、不再把并发压到 1-2——
        // 16 GB 档 cap 4、32 GB 档 cap 3，仍保留可用并发（见上方 cap 断言）。
        // cap 与文件数分档的交互：cap 不可能把并发抬升，只会压低。
        let make = |n: usize| {
            (0..n)
                .map(|i| format!("__missing_{i}.cli"))
                .collect::<Vec<_>>()
        };
        // 无大文件时 cap = AUTO_MAX(6)，分档结果即最终值
        assert_eq!(determine_file_workers(&make(8), 0), 4);
        assert_eq!(determine_file_workers(&make(20), 0), 6);
        // 手动指定也不超过 size cap（缺失文件按 0 字节 → cap 6）
        assert_eq!(determine_file_workers(&make(20), 64), 6);
    }
}
