use tauri::{AppHandle, Window};

pub fn open_print_window(app: &AppHandle, job_id: &str, job_name: &str) -> Result<(), String> {
    crate::webview2_system_print::open_print_window(app, job_id, job_name)
}

pub fn trigger_system_print_dialog(window: &Window) -> Result<(), String> {
    crate::webview2_system_print::trigger_system_print_dialog(window)
}
