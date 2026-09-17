mod merge;
mod parameter;
mod runtime;
mod split;

use merge::{MergeRequest, MergeResult};
use parameter::ParameterSnapshotDto;
use split::{SplitRequest, SplitResult};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

static TASK_RUNNING: AtomicBool = AtomicBool::new(false);

struct TaskGuard;
impl TaskGuard {
    fn acquire() -> Result<Self, String> {
        TASK_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "已有 CLI 任务正在运行，请等待当前任务完成。".to_string())
    }
}
impl Drop for TaskGuard {
    fn drop(&mut self) {
        TASK_RUNNING.store(false, Ordering::Release);
    }
}

#[tauri::command]
async fn load_parameter_excel(app: AppHandle, path: String) -> Result<ParameterSnapshotDto, String> {
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || parameter::load_parameter_excel(&worker_app, &path))
        .await
        .map_err(|e| format!("参数加载任务异常退出：{e}"))?;
    match &result {
        Ok(v) => runtime::log(&app, "info", &format!("参数文件加载成功：{}", v.source_name)),
        Err(e) => runtime::log(&app, "error", &format!("参数文件加载失败：{e}")),
    }
    result
}

#[tauri::command]
fn load_saved_parameter_snapshot(app: AppHandle) -> Result<Option<ParameterSnapshotDto>, String> {
    parameter::load_saved_parameter_snapshot(&app)
}

#[tauri::command]
async fn split_cli_files(app: AppHandle, request: SplitRequest) -> Result<SplitResult, String> {
    let _guard = TaskGuard::acquire()?;
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || split::split_cli_files(&worker_app, request))
        .await
        .map_err(|e| format!("分割任务异常退出：{e}"))?;
    if let Err(e) = &result { runtime::log(&app, "error", &format!("CLI 分割失败：{e}")); }
    result
}

#[tauri::command]
async fn merge_cli_files(app: AppHandle, request: MergeRequest) -> Result<MergeResult, String> {
    let _guard = TaskGuard::acquire()?;
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || merge::merge_cli_files(&worker_app, request))
        .await
        .map_err(|e| format!("合并任务异常退出：{e}"))?;
    if let Err(e) = &result { runtime::log(&app, "error", &format!("CLI 合并失败：{e}")); }
    result
}


#[tauri::command]
fn apply_runtime_settings(app: AppHandle, settings: runtime::RuntimeSettings) -> Result<(), String> {
    runtime::apply_settings(&app, settings)
}

#[tauri::command]
fn get_storage_info(app: AppHandle) -> Result<runtime::StorageInfo, String> {
    runtime::storage_info(&app)
}

#[tauri::command]
fn clear_cache(app: AppHandle) -> Result<u64, String> {
    runtime::clear_cache(&app)
}

#[tauri::command]
fn clear_logs(app: AppHandle) -> Result<u64, String> {
    runtime::clear_logs(&app)
}


#[tauri::command]
fn install_ui_image(app: AppHandle, kind: String, path: String) -> Result<String, String> {
    runtime::install_ui_image(&app, &kind, &path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_parameter_excel,
            load_saved_parameter_snapshot,
            split_cli_files,
            merge_cli_files,
            apply_runtime_settings,
            get_storage_info,
            clear_cache,
            clear_logs,
            install_ui_image
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) && runtime::current_settings().auto_clear_cache {
                let _ = runtime::clear_cache(app);
            }
        });
}
