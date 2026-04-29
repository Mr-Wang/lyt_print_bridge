use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(not(target_os = "windows"))]
use tauri::api::shell;
#[cfg(not(target_os = "windows"))]
use tauri::Manager;
use tauri::{AppHandle, State, Window};

use crate::{
    app_state::{
        emit_snapshot, persist_settings, AppState, PrintJobDocument, PrintJobStatus,
        RuntimeSnapshot, SaveSettingsInput,
    },
    bridge_http, print_service, ServiceState,
};

#[tauri::command]
pub fn get_runtime_snapshot(state: State<'_, AppState>) -> RuntimeSnapshot {
    state.core.snapshot()
}

#[tauri::command]
pub fn restart_local_service(
    app: AppHandle,
    state: State<'_, AppState>,
    service: State<'_, ServiceState>,
) -> Result<RuntimeSnapshot, String> {
    bridge_http::restart_service(&app, &state, &service)?;
    crate::diagnostics::write(&app, "commands", "local http service restarted by user");
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn get_log_file_path(app: AppHandle) -> String {
    crate::diagnostics::log_file(&app)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub fn get_log_dir_path(app: AppHandle) -> String {
    crate::diagnostics::log_dir(&app)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub fn get_desktop_log_file_path() -> Option<String> {
    crate::diagnostics::desktop_log_file().map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_file = crate::diagnostics::ensure_log_file(&app)?;
    let log_dir = log_file
        .parent()
        .ok_or_else(|| "日志目录路径无效".to_string())?;
    open_path(&app, log_dir)
}

#[tauri::command]
pub fn open_log_file(app: AppHandle) -> Result<(), String> {
    let log_file = crate::diagnostics::ensure_log_file(&app)?;
    open_file(&app, &log_file)
}

#[tauri::command]
pub fn open_desktop_log_file(app: AppHandle) -> Result<(), String> {
    crate::diagnostics::ensure_log_file(&app)?;
    let log_file = crate::diagnostics::desktop_log_file()
        .ok_or_else(|| "未能定位桌面日志文件路径".to_string())?;
    open_file(&app, &log_file)
}

#[tauri::command]
pub fn frontend_log(app: AppHandle, source: String, message: String) -> Result<(), String> {
    crate::diagnostics::try_write(&app, source, message)
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    service: State<'_, ServiceState>,
    input: SaveSettingsInput,
) -> Result<RuntimeSnapshot, String> {
    state.core.apply_settings_input(input)?;
    persist_settings(state.core.as_ref())?;
    bridge_http::restart_service(&app, &state, &service)?;
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn rotate_access_token(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeSnapshot, String> {
    state.core.rotate_access_token();
    persist_settings(state.core.as_ref())?;
    emit_snapshot(&app, &state.core);
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn approve_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<RuntimeSnapshot, String> {
    print_service::approve_job(&app, &state.core, &job_id)?;
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn cancel_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<RuntimeSnapshot, String> {
    print_service::cancel_job(&app, &state.core, &job_id)?;
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn close_current_window(window: Window, app: AppHandle) -> Result<(), String> {
    crate::diagnostics::write(
        &app,
        "commands",
        format!("close_current_window window_label={}", window.label()),
    );
    window
        .close()
        .map_err(|error| format!("关闭当前窗口失败: {error}"))
}

#[tauri::command]
pub fn hide_current_window(window: Window, app: AppHandle) -> Result<(), String> {
    crate::diagnostics::write(
        &app,
        "commands",
        format!("hide_current_window window_label={}", window.label()),
    );
    window
        .hide()
        .map_err(|error| format!("隐藏当前窗口失败: {error}"))
}

#[tauri::command]
pub fn open_bridge_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let config_dir = state.core.config_dir();
    fs::create_dir_all(&config_dir).map_err(|error| format!("创建配置目录失败: {error}"))?;
    open_path(&app, &config_dir)
}

#[tauri::command]
pub fn list_local_printers() -> Result<Vec<String>, String> {
    print_service::list_printers()
}

#[tauri::command]
pub fn detect_sumatra_pdf_path(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeSnapshot, String> {
    let mut next = state.core.settings();
    let detected =
        print_service::detect_sumatra_pdf_path(&next.sumatra_pdf_path).ok_or_else(|| {
            "未检测到 SumatraPDF.exe。请先安装 SumatraPDF，或手动填写完整路径。".to_string()
        })?;
    next.sumatra_pdf_path = detected;
    state.core.replace_settings(next);
    persist_settings(state.core.as_ref())?;
    emit_snapshot(&app, &state.core);
    Ok(state.core.snapshot())
}

#[tauri::command]
pub fn get_print_job_document(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<PrintJobDocument, String> {
    crate::diagnostics::write(
        &app,
        "commands",
        format!("get_print_job_document requested job_id={job_id}"),
    );
    let job = state
        .core
        .find_job(&job_id)
        .ok_or_else(|| format!("未找到打印任务: {job_id}"))?;
    let local_file_path = job
        .local_file_path
        .clone()
        .ok_or_else(|| format!("打印任务缺少本地 PDF 文件: {job_id}"))?;

    let file_path = PathBuf::from(&local_file_path);
    if !file_path.exists() {
        return Err(format!("本地 PDF 文件不存在: {}", file_path.display()));
    }

    Ok(PrintJobDocument {
        id: job.id,
        job_name: job.job_name,
        local_file_path,
        printer_name: job.printer_name,
        copies: job.copies,
        page_range: job.page_range,
    })
}

#[tauri::command]
pub fn notify_print_ready(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    crate::diagnostics::write(
        &app,
        "commands",
        format!(
            "notify_print_ready received job_id={} window_label={}",
            job_id,
            window.label()
        ),
    );
    let result = crate::platform_print::trigger_system_print_dialog(&window);

    match result {
        Ok(()) => {
            crate::diagnostics::write(
                &app,
                "commands",
                format!("system print dialog opened job_id={job_id}"),
            );
            state.core.update_job(&job_id, |job| {
                job.status = PrintJobStatus::DialogOpened;
                job.error = None;
            })?;
            emit_snapshot(&app, &state.core);
            Ok(())
        }
        Err(error) => {
            crate::diagnostics::write(
                &app,
                "commands",
                format!(
                    "system print dialog failed job_id={} error={}",
                    job_id, error
                ),
            );
            state.core.update_job(&job_id, |job| {
                job.status = PrintJobStatus::Failed;
                job.error = Some(error.clone());
            })?;
            emit_snapshot(&app, &state.core);
            Err(error)
        }
    }
}

#[tauri::command]
pub fn report_print_error(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    message: String,
) -> Result<(), String> {
    crate::diagnostics::write(
        &app,
        "commands",
        format!("report_print_error job_id={} message={}", job_id, message),
    );
    state.core.update_job(&job_id, |job| {
        job.status = PrintJobStatus::Failed;
        job.error = Some(message.clone());
    })?;
    emit_snapshot(&app, &state.core);
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_path(_app: &AppHandle, path: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("打开目录失败: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn open_path(app: &AppHandle, path: &Path) -> Result<(), String> {
    shell::open(
        &app.shell_scope(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .map_err(|error| format!("打开目录失败: {error}"))
}

#[cfg(target_os = "windows")]
fn open_file(_app: &AppHandle, path: &Path) -> Result<(), String> {
    Command::new("notepad.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("打开日志文件失败: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn open_file(app: &AppHandle, path: &Path) -> Result<(), String> {
    shell::open(
        &app.shell_scope(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .map_err(|error| format!("打开日志文件失败: {error}"))
}
