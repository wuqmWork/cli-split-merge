use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettings {
    pub log_level: String,
    pub log_retention_days: u64,
    pub auto_clear_cache: bool,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            log_retention_days: 14,
            auto_clear_cache: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub log_dir: String,
    pub cache_dir: String,
    pub log_bytes: u64,
    pub cache_bytes: u64,
}

static SETTINGS: OnceLock<RwLock<RuntimeSettings>> = OnceLock::new();
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn settings_lock() -> &'static RwLock<RuntimeSettings> {
    SETTINGS.get_or_init(|| RwLock::new(RuntimeSettings::default()))
}

pub fn apply_settings(app: &AppHandle, settings: RuntimeSettings) -> Result<(), String> {
    *settings_lock()
        .write()
        .map_err(|_| "运行设置锁异常".to_string())? = settings.clone();
    purge_old_logs(app, settings.log_retention_days)?;
    log(app, "info", "运行设置已更新");
    Ok(())
}

pub fn current_settings() -> RuntimeSettings {
    settings_lock()
        .read()
        .map(|v| v.clone())
        .unwrap_or_default()
}

pub fn log(app: &AppHandle, level: &str, message: &str) {
    let cfg = current_settings();
    if level_rank(level) > level_rank(&cfg.log_level) {
        return;
    }
    let _guard = LOG_LOCK.get_or_init(|| Mutex::new(())).lock().ok();
    let Ok(dir) = log_dir(app) else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join(format!("app-{}.log", Utc::now().format("%Y-%m-%d")));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(
            f,
            "{} [{:>5}] {}",
            Utc::now().to_rfc3339(),
            level.to_uppercase(),
            message
        );
    }
}

pub fn write_cache(app: &AppHandle, name: &str, content: &str) {
    let Ok(dir) = cache_dir(app) else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let safe = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let _ = fs::write(dir.join(format!("{safe}.json")), content.as_bytes());
}

pub fn install_ui_image(app: &AppHandle, kind: &str, source: &str) -> Result<String, String> {
    let source_path = PathBuf::from(source);
    if !source_path.is_file() {
        return Err(format!("图片文件不存在：{}", source_path.display()));
    }
    let ext = source_path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp") {
        return Err("仅支持 png/jpg/jpeg/webp/bmp 图片".to_string());
    }
    let file_stem = match kind {
        "logo" => "logo",
        "splash" => "splash",
        _ => return Err("未知图片类型".to_string()),
    };
    let dir = base_data_dir(app)?.join("assets");
    fs::create_dir_all(&dir).map_err(|e| format!("创建图片资源目录失败：{e}"))?;
    // 删除同类旧资源，避免切换扩展名后残留。
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(file_stem) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let target = dir.join(format!("{file_stem}.{ext}"));
    fs::copy(&source_path, &target).map_err(|e| format!("复制图片资源失败：{e}"))?;
    log(app, "info", &format!("已更新界面图片：{kind}"));
    Ok(target.to_string_lossy().into_owned())
}

pub fn storage_info(app: &AppHandle) -> Result<StorageInfo, String> {
    let logs = log_dir(app)?;
    let cache = cache_dir(app)?;
    Ok(StorageInfo {
        log_dir: logs.to_string_lossy().into_owned(),
        cache_dir: cache.to_string_lossy().into_owned(),
        log_bytes: dir_size(&logs),
        cache_bytes: dir_size(&cache),
    })
}

pub fn clear_cache(app: &AppHandle) -> Result<u64, String> {
    let dir = cache_dir(app)?;
    let before = dir_size(&dir);
    clear_dir(&dir)?;
    log(app, "info", &format!("已清理缓存 {} 字节", before));
    Ok(before)
}

pub fn clear_logs(app: &AppHandle) -> Result<u64, String> {
    let dir = log_dir(app)?;
    let before = dir_size(&dir);
    clear_dir(&dir)?;
    // 清理后不立即写日志，避免界面显示刚清空又出现新文件。
    Ok(before)
}

pub fn purge_old_logs(app: &AppHandle, days: u64) -> Result<(), String> {
    if days == 0 {
        return Ok(());
    }
    let dir = log_dir(app)?;
    if !dir.exists() {
        return Ok(());
    }
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days.saturating_mul(86_400)))
        .unwrap_or(UNIX_EPOCH);
    let entries = fs::read_dir(&dir).map_err(|e| format!("读取日志目录失败：{e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::now());
        if modified < cutoff {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn level_rank(level: &str) -> u8 {
    match level.to_ascii_lowercase().as_str() {
        "error" => 0,
        "warn" => 1,
        "info" => 2,
        "debug" => 3,
        _ => 2,
    }
}
fn base_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法获取程序数据目录：{e}"))
}
fn log_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(base_data_dir(app)?.join("logs"))
}
fn cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(base_data_dir(app)?.join("cache"))
}
fn clear_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("清理目录 {} 失败：{e}", dir.display()))?;
    }
    fs::create_dir_all(dir).map_err(|e| format!("创建目录 {} 失败：{e}", dir.display()))
}
fn dir_size(path: &Path) -> u64 {
    if path.is_file() {
        return fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().map(|e| dir_size(&e.path())).sum()
}
