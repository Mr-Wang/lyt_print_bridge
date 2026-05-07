mod app_state;
mod autostart;
mod bridge_http;
mod commands;
mod diagnostics;
mod platform_print;
mod print_service;
mod webview2_system_print;

use std::sync::Mutex;
use tauri::{CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, WindowEvent};

use app_state::{emit_snapshot, load_state, AppState};

#[derive(Default)]
pub struct ServiceState {
    pub handle: Mutex<Option<bridge_http::LocalHttpHandle>>,
}

pub fn run() {
    diagnostics::bootstrap("process started");
    let tray_menu = SystemTrayMenu::new()
        .add_item(CustomMenuItem::new("open_status", "打开状态"))
        .add_item(CustomMenuItem::new("open_log", "打开日志"))
        .add_item(CustomMenuItem::new("quit", "退出"));

    let builder = tauri::Builder::default()
        .system_tray(SystemTray::new().with_menu(tray_menu))
        .setup(|app| {
            diagnostics::bootstrap("setup entered");
            autostart::cleanup_legacy_processes();
            let core = match load_state(&app.handle()) {
                Ok(core) => core,
                Err(error) => {
                    diagnostics::bootstrap(format!("load_state failed: {error}"));
                    return Err(error.into());
                }
            };
            let app_state = AppState::new(core);
            app.manage(app_state);
            app.manage(ServiceState::default());

            let managed_state: tauri::State<'_, AppState> = app.state();
            diagnostics::write(
                &app.handle(),
                "app",
                format!(
                    "setup completed config_dir={} log_file={}",
                    managed_state.core.config_dir().display(),
                    diagnostics::log_file(&app.handle()).display()
                ),
            );
            let service_state: tauri::State<'_, ServiceState> = app.state();
            match bridge_http::restart_service(&app.handle(), &managed_state, &service_state) {
                Ok(()) => diagnostics::write(&app.handle(), "app", "local http service restarted"),
                Err(error) => diagnostics::write(
                    &app.handle(),
                    "app",
                    format!("local http service failed: {error}"),
                ),
            }
            emit_snapshot(&app.handle(), &managed_state.core);
            match autostart::ensure_enabled() {
                Ok(()) => diagnostics::write(&app.handle(), "app", "autostart ensured"),
                Err(error) => diagnostics::write(
                    &app.handle(),
                    "app",
                    format!("autostart setup failed: {error}"),
                ),
            }

            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_window("main") {
                    window.open_devtools();
                }
            }

            Ok(())
        })
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick { .. } | SystemTrayEvent::DoubleClick { .. } => {
                show_status_window(app);
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "open_status" => show_status_window(app),
                "open_log" => {
                    if let Err(error) = commands::open_log_file(app.clone()) {
                        diagnostics::write(app, "tray", format!("open_log failed: {error}"));
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            },
            _ => {}
        })
        .on_window_event(|event| {
            if event.window().label() != "main" {
                return;
            }

            if let WindowEvent::CloseRequested { api, .. } = event.event() {
                let _ = event.window().hide();
                api.prevent_close();
            }
        })
        .on_page_load(|window, payload| {
            diagnostics::write(
                &window.app_handle(),
                "page_load",
                format!("label={} url={}", window.label(), payload.url()),
            );
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_snapshot,
            commands::restart_local_service,
            commands::save_settings,
            commands::rotate_access_token,
            commands::approve_job,
            commands::cancel_job,
            commands::close_current_window,
            commands::hide_current_window,
            commands::open_bridge_dir,
            commands::open_log_dir,
            commands::open_log_file,
            commands::open_desktop_log_file,
            commands::list_local_printers,
            commands::detect_sumatra_pdf_path,
            commands::get_print_job_document,
            commands::notify_print_ready,
            commands::report_print_error,
            commands::frontend_log,
            commands::get_log_file_path,
            commands::get_log_dir_path,
            commands::get_desktop_log_file_path,
        ]);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        panic!("error while running lyt_print_bridge: {error}");
    }
}

fn show_status_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
