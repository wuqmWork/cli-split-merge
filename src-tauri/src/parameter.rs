use calamine::{open_workbook_auto, Data, Reader};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

const SHEET_NAME: &str = "C3到各振镜";
const SNAPSHOT_FILE: &str = "parameter_snapshot.json";
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorTransform {
    pub category: String,
    pub transform_name: String,
    pub source: String,
    pub target: String,
    pub point_count: u32,
    pub a: f64,
    pub b: f64,
    pub tx: f64,
    pub ty: f64,
    pub rotation_deg: f64,
    pub matrix: [[f64; 3]; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSnapshotDto {
    pub source_path: String,
    pub source_name: String,
    pub loaded_at: String,
    pub schema_version: u32,
    pub sheet_name: String,
    pub transform_count: usize,
    pub transforms: Vec<MirrorTransform>,
}

pub fn load_parameter_excel(app: &AppHandle, path: &str) -> Result<ParameterSnapshotDto, String> {
    let source_path = PathBuf::from(path);
    if !source_path.is_file() {
        return Err(format!("参数文件不存在：{}", source_path.display()));
    }

    let mut workbook =
        open_workbook_auto(&source_path).map_err(|e| format!("无法打开 Excel 参数文件：{e}"))?;

    let range = workbook
        .worksheet_range(SHEET_NAME)
        .map_err(|e| format!("读取工作表“{SHEET_NAME}”失败：{e}"))?;

    let mut rows = range.rows();
    let header = rows
        .next()
        .ok_or_else(|| format!("工作表“{SHEET_NAME}”为空"))?;
    let columns = header_map(header)?;

    // 两种格式：
    // 完整格式：类别/变换名称/旋转角deg/m11~m33 齐全；
    // 精简格式：仅 源坐标系/目标坐标系/点数/a/b/tx/ty，旋转角与矩阵由 a/b 推导。
    let simple = is_simple_format(&columns);

    for required in ["源坐标系", "目标坐标系", "a", "b", "tx", "ty"] {
        if !columns.contains_key(required) {
            return Err(format!("参数文件格式不完整：缺少列“{required}”"));
        }
    }
    if !simple {
        for required in [
            "类别",
            "变换名称",
            "点数",
            "旋转角deg",
            "m11",
            "m12",
            "m13",
            "m21",
            "m22",
            "m23",
            "m31",
            "m32",
            "m33",
        ] {
            if !columns.contains_key(required) {
                return Err(format!("参数文件格式不完整：缺少列“{required}”"));
            }
        }
    }

    let mut by_target: BTreeMap<String, MirrorTransform> = BTreeMap::new();
    for (row_offset, row) in rows.enumerate() {
        let row_no = row_offset + 2;
        let target = text_at(row, &columns, "目标坐标系")?;
        if target.trim().is_empty() {
            continue;
        }

        let source = text_at(row, &columns, "源坐标系")?;
        if source != "C3" {
            return Err(format!("第 {row_no} 行源坐标系应为 C3，实际为“{source}”"));
        }

        let transform = if simple {
            parse_simple_row(row, &columns, &source, &target)?
        } else {
            parse_full_row(row, &columns, &source, &target)?
        };

        validate_transform(&transform, row_no)?;
        if by_target.insert(target.clone(), transform).is_some() {
            return Err(format!("参数文件中目标坐标系“{target}”重复"));
        }
    }

    validate_mirror_set(by_target.keys().cloned().collect())?;
    let transforms: Vec<MirrorTransform> = by_target.into_values().collect();

    let snapshot = ParameterSnapshotDto {
        source_path: source_path.to_string_lossy().to_string(),
        source_name: source_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("params.xlsx")
            .to_string(),
        loaded_at: Utc::now().to_rfc3339(),
        schema_version: SCHEMA_VERSION,
        sheet_name: SHEET_NAME.to_string(),
        transform_count: transforms.len(),
        transforms,
    };

    persist_snapshot(app, &snapshot)?;
    Ok(snapshot)
}

fn is_simple_format(columns: &HashMap<String, usize>) -> bool {
    let full_markers = ["类别", "旋转角deg", "m11"];
    if full_markers.iter().all(|c| columns.contains_key(*c)) {
        return false;
    }
    true
}

fn number_or_zero(row: &[Data], columns: &HashMap<String, usize>, name: &str) -> f64 {
    let Some(&index) = columns.get(name) else {
        return 0.0;
    };
    let Some(cell) = row.get(index) else {
        return 0.0;
    };
    cell_number(cell).filter(|v| v.is_finite()).unwrap_or(0.0)
}

fn parse_full_row(
    row: &[Data],
    columns: &HashMap<String, usize>,
    source: &str,
    target: &str,
) -> Result<MirrorTransform, String> {
    Ok(MirrorTransform {
        category: text_at(row, columns, "类别")?,
        transform_name: text_at(row, columns, "变换名称")?,
        source: source.to_string(),
        target: target.to_string(),
        point_count: number_at(row, columns, "点数")?.round().max(0.0) as u32,
        a: number_at(row, columns, "a")?,
        b: number_at(row, columns, "b")?,
        tx: number_at(row, columns, "tx")?,
        ty: number_at(row, columns, "ty")?,
        rotation_deg: number_at(row, columns, "旋转角deg")?,
        matrix: [
            [
                number_at(row, columns, "m11")?,
                number_at(row, columns, "m12")?,
                number_at(row, columns, "m13")?,
            ],
            [
                number_at(row, columns, "m21")?,
                number_at(row, columns, "m22")?,
                number_at(row, columns, "m23")?,
            ],
            [
                number_at(row, columns, "m31")?,
                number_at(row, columns, "m32")?,
                number_at(row, columns, "m33")?,
            ],
        ],
    })
}

/// 精简格式：只有 a/b/tx/ty，旋转角与 3×3 矩阵按“说明”页公式推导：
/// x' = a*x - b*y + tx；y' = b*x + a*y + ty
fn parse_simple_row(
    row: &[Data],
    columns: &HashMap<String, usize>,
    source: &str,
    target: &str,
) -> Result<MirrorTransform, String> {
    let a = number_at(row, columns, "a")?;
    let b = number_at(row, columns, "b")?;
    let tx = number_at(row, columns, "tx")?;
    let ty = number_at(row, columns, "ty")?;
    Ok(MirrorTransform {
        category: SHEET_NAME.to_string(),
        transform_name: format!("T_C3_to_{target}"),
        source: source.to_string(),
        target: target.to_string(),
        point_count: number_or_zero(row, columns, "点数").round().max(0.0) as u32,
        a,
        b,
        tx,
        ty,
        rotation_deg: b.atan2(a).to_degrees(),
        matrix: [[a, -b, tx], [b, a, ty], [0.0, 0.0, 1.0]],
    })
}

pub fn load_saved_parameter_snapshot(
    app: &AppHandle,
) -> Result<Option<ParameterSnapshotDto>, String> {
    let path = snapshot_path(app)?;
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(&path).map_err(|e| format!("读取参数快照失败：{e}"))?;
    let snapshot: ParameterSnapshotDto =
        serde_json::from_slice(&bytes).map_err(|e| format!("参数快照格式无效：{e}"))?;

    if snapshot.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "参数快照版本不兼容：当前 {}，文件 {}",
            SCHEMA_VERSION, snapshot.schema_version
        ));
    }
    if snapshot.transform_count != 24 || snapshot.transforms.len() != 24 {
        return Err("参数快照不完整：必须包含 24 个振镜变换".to_string());
    }

    for (index, transform) in snapshot.transforms.iter().enumerate() {
        if !transform.source.trim().eq_ignore_ascii_case("C3") {
            return Err(format!("参数快照第 {} 条源坐标系不是 C3", index + 1));
        }
        validate_transform(transform, index + 1)?;
    }
    let targets: BTreeSet<String> = snapshot
        .transforms
        .iter()
        .map(|v| v.target.clone())
        .collect();
    validate_mirror_set(targets)?;
    Ok(Some(snapshot))
}

fn persist_snapshot(app: &AppHandle, snapshot: &ParameterSnapshotDto) -> Result<(), String> {
    let path = snapshot_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建参数快照目录失败：{e}"))?;
    }

    let temp = path.with_extension("json.tmp");
    let payload =
        serde_json::to_vec_pretty(snapshot).map_err(|e| format!("序列化参数快照失败：{e}"))?;
    fs::write(&temp, payload).map_err(|e| format!("写入参数快照失败：{e}"))?;
    fs::rename(&temp, &path).map_err(|e| format!("保存参数快照失败：{e}"))?;
    Ok(())
}

fn snapshot_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法获取程序数据目录：{e}"))?;
    Ok(dir.join(SNAPSHOT_FILE))
}

fn header_map(header: &[Data]) -> Result<HashMap<String, usize>, String> {
    let mut map = HashMap::new();
    for (index, cell) in header.iter().enumerate() {
        if let Some(value) = cell_text(cell) {
            let key = value.trim().to_string();
            if !key.is_empty() {
                map.insert(key, index);
            }
        }
    }
    if map.is_empty() {
        return Err("无法识别参数表表头".to_string());
    }
    Ok(map)
}

fn text_at(row: &[Data], columns: &HashMap<String, usize>, name: &str) -> Result<String, String> {
    let index = *columns.get(name).ok_or_else(|| format!("缺少列“{name}”"))?;
    let cell = row.get(index).ok_or_else(|| format!("列“{name}”无数据"))?;
    cell_text(cell).ok_or_else(|| format!("列“{name}”不是有效文本"))
}

fn number_at(row: &[Data], columns: &HashMap<String, usize>, name: &str) -> Result<f64, String> {
    let index = *columns.get(name).ok_or_else(|| format!("缺少列“{name}”"))?;
    let cell = row.get(index).ok_or_else(|| format!("列“{name}”无数据"))?;
    let value = cell_number(cell).ok_or_else(|| format!("列“{name}”不是有效数值"))?;
    if !value.is_finite() {
        return Err(format!("列“{name}”包含非有限数值"));
    }
    Ok(value)
}

fn cell_text(cell: &Data) -> Option<String> {
    match cell {
        Data::String(v) => Some(v.clone()),
        Data::Int(v) => Some(v.to_string()),
        Data::Float(v) => Some(v.to_string()),
        Data::Bool(v) => Some(v.to_string()),
        Data::Empty => Some(String::new()),
        _ => None,
    }
}

fn cell_number(cell: &Data) -> Option<f64> {
    match cell {
        Data::Float(v) => Some(*v),
        Data::Int(v) => Some(*v as f64),
        Data::String(v) => v.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn validate_transform(transform: &MirrorTransform, row_no: usize) -> Result<(), String> {
    let values = [
        transform.a,
        transform.b,
        transform.tx,
        transform.ty,
        transform.rotation_deg,
        transform.matrix[0][0],
        transform.matrix[0][1],
        transform.matrix[0][2],
        transform.matrix[1][0],
        transform.matrix[1][1],
        transform.matrix[1][2],
        transform.matrix[2][0],
        transform.matrix[2][1],
        transform.matrix[2][2],
    ];
    if values.iter().any(|v| !v.is_finite()) {
        return Err(format!("第 {row_no} 行存在非法数值"));
    }

    // 刚体旋转中 a²+b² 应接近 1。留出宽松容差，仅拦截明显损坏的参数文件。
    let norm = transform.a * transform.a + transform.b * transform.b;
    if (norm - 1.0).abs() > 0.02 {
        return Err(format!(
            "第 {row_no} 行 {} 的旋转参数异常：a²+b²={norm:.6}",
            transform.target
        ));
    }
    Ok(())
}

fn validate_mirror_set(actual: BTreeSet<String>) -> Result<(), String> {
    let expected: BTreeSet<String> = ["A", "B", "C", "D", "E", "F"]
        .into_iter()
        .flat_map(|group| (0..=3).map(move |index| format!("{group}{index}")))
        .collect();

    if actual == expected {
        return Ok(());
    }

    let missing: Vec<_> = expected.difference(&actual).cloned().collect();
    let extra: Vec<_> = actual.difference(&expected).cloned().collect();
    Err(format!(
        "参数文件振镜集合不完整。缺少：[{}]；多余：[{}]",
        missing.join(", "),
        extra.join(", ")
    ))
}

#[allow(dead_code)]
pub fn apply_transform(t: &MirrorTransform, x: f64, y: f64) -> (f64, f64) {
    // Excel“说明”页定义：
    // x_target = a*x_source - b*y_source + tx
    // y_target = b*x_source + a*y_source + ty
    (t.a * x - t.b * y + t.tx, t.b * x + t.a * y + t.ty)
}

#[allow(dead_code)]
pub fn snapshot_file_for_test(base: &Path) -> PathBuf {
    base.join(SNAPSHOT_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(target: &str) -> MirrorTransform {
        MirrorTransform {
            category: "test".into(),
            transform_name: format!("C3->{target}"),
            source: "C3".into(),
            target: target.into(),
            point_count: 4,
            a: 1.0,
            b: 0.0,
            tx: 0.0,
            ty: 0.0,
            rotation_deg: 0.0,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    #[test]
    fn mirror_set_requires_all_24() {
        let all: BTreeSet<String> = ["A", "B", "C", "D", "E", "F"]
            .into_iter()
            .flat_map(|g| (0..=3).map(move |i| format!("{g}{i}")))
            .collect();
        assert!(validate_mirror_set(all).is_ok());

        let mut missing: BTreeSet<String> = ["A", "B", "C", "D", "E", "F"]
            .into_iter()
            .flat_map(|g| (0..=3).map(move |i| format!("{g}{i}")))
            .collect();
        missing.remove("F3");
        assert!(validate_mirror_set(missing).is_err());
    }

    #[test]
    fn transform_norm_is_revalidated() {
        assert!(validate_transform(&identity("C3"), 1).is_ok());
        let mut broken = identity("C3");
        broken.a = 2.0;
        assert!(validate_transform(&broken, 1).is_err());
    }
}
