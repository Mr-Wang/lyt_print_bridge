mod app_state;
mod autostart;
mod bridge_http;
mod commands;
mod diagnostics;
mod platform_print;
mod print_service;
mod webview2_system_print;

use std::{panic, sync::Mutex};
use tauri::{
    api::shell, CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu,
    WindowBuilder, WindowEvent, WindowUrl,
};

use app_state::{emit_snapshot, load_state, AppState};

const DIAGNOSTIC_EVAL_SCRIPT: &str = r#"
(function () {
  var prefix = '[lyt-diagnostic]';

  function paintMarker() {
    console.log(prefix, 'eval reached', {
      href: window.location.href,
      readyState: document.readyState,
      hasTauriIpc: Boolean(window.__TAURI_IPC__),
      userAgent: navigator.userAgent
    });

    document.documentElement.style.background = '#fff7ed';
    if (!document.body) {
      console.warn(prefix, 'document.body is not ready');
      return;
    }

    document.body.style.background = '#fff7ed';
    document.body.style.minHeight = '100vh';

    var marker = document.getElementById('lyt-diagnostic-marker');
    if (!marker) {
      marker = document.createElement('div');
      marker.id = 'lyt-diagnostic-marker';
      document.body.appendChild(marker);
    }

    marker.textContent = 'v0.1.17 诊断脚本已执行 - 请打开 Console 查看红色报错';
    marker.style.cssText = [
      'position:fixed',
      'left:8px',
      'right:8px',
      'bottom:8px',
      'z-index:2147483647',
      'background:#b91c1c',
      'color:#fff',
      'padding:8px 10px',
      'font:12px Microsoft YaHei, Segoe UI, sans-serif',
      'box-shadow:0 6px 18px rgba(0,0,0,.18)',
      'border-radius:4px',
      'text-align:center'
    ].join(';');
  }

  window.addEventListener('error', function (event) {
    console.error(prefix, 'window error', event.message, event.filename, event.lineno, event.colno, event.error);
  });
  window.addEventListener('unhandledrejection', function (event) {
    console.error(prefix, 'unhandled rejection', event.reason);
  });

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', paintMarker, { once: true });
  } else {
    paintMarker();
  }
})();
"#;

#[derive(Default)]
pub struct ServiceState {
    pub handle: Mutex<Option<bridge_http::LocalHttpHandle>>,
}

pub fn run() {
    diagnostics::bootstrap("process started");
    install_panic_hook();
    diagnostics::bootstrap("panic hook installed");

    let tray_menu = SystemTrayMenu::new()
        .add_item(CustomMenuItem::new("open_browser_status", "浏览器状态页"))
        .add_item(CustomMenuItem::new("open_status", "打开内置窗口"))
        .add_item(CustomMenuItem::new("open_devtools", "打开内置控制台"))
        .add_item(CustomMenuItem::new("open_log", "打开日志"))
        .add_item(CustomMenuItem::new("quit", "退出"));
    diagnostics::bootstrap("tray menu created");

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

            diagnostics::write(
                &app.handle(),
                "diagnostics",
                "embedded status window skipped on startup; using browser status page",
            );
            log_webview2_runtime_hint(&app.handle());
            open_browser_status_page(&app.handle(), "setup");

            Ok(())
        })
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick { .. } | SystemTrayEvent::DoubleClick { .. } => {
                open_browser_status_page(app, "tray_click");
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "open_browser_status" => open_browser_status_page(app, "tray_menu"),
                "open_status" => show_status_window(app),
                "open_devtools" => open_main_devtools(app),
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
            log_window_state(&window, "page_load");
            if window.label() == "main" {
                inject_page_diagnostics(&window, "page_load");
            }
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

    diagnostics::bootstrap("builder configured, run starting");
    if let Err(error) = builder.run(tauri::generate_context!()) {
        diagnostics::bootstrap(format!("builder.run returned error: {error}"));
        panic!("error while running lyt_print_bridge: {error}");
    }
}

fn show_status_window(app: &tauri::AppHandle) {
    match app.get_window("main") {
        Some(window) => focus_status_window(&window),
        None => match create_status_window(app, "show_status") {
            Ok(window) => focus_status_window(&window),
            Err(error) => diagnostics::write(
                app,
                "tray",
                format!("show_status failed to create window: {error}"),
            ),
        },
    }
}

fn open_main_devtools(app: &tauri::AppHandle) {
    if let Some(window) = app.get_window("main") {
        show_status_window(app);
        window.open_devtools();
        diagnostics::write(app, "tray", "main devtools opened by tray menu");
        inject_page_diagnostics(&window, "tray_open_devtools");
    } else {
        diagnostics::write(app, "tray", "open_devtools failed: main window not found");
    }
}

fn create_status_window(
    app: &tauri::AppHandle,
    source: &str,
) -> Result<tauri::Window, tauri::Error> {
    diagnostics::write(
        app,
        "window",
        format!("{source} creating main window from status.html"),
    );

    let window = WindowBuilder::new(app, "main", WindowUrl::App("status.html".into()))
        .title("辽易通打印桥 v0.1.17")
        .inner_size(360.0, 220.0)
        .min_inner_size(360.0, 220.0)
        .resizable(false)
        .fullscreen(false)
        .visible(true)
        .build()?;

    diagnostics::write(app, "window", format!("{source} main window created"));
    Ok(window)
}

fn open_browser_status_page(app: &tauri::AppHandle, source: &str) {
    let url = match app.try_state::<AppState>() {
        Some(state) => state.core.service_status().base_url + "/status",
        None => format!("http://127.0.0.1:{}/status", app_state::FIXED_PORT),
    };

    diagnostics::write(
        app,
        "browser_status",
        format!("{source} opening status page url={url}"),
    );

    if let Err(error) = shell::open(&app.shell_scope(), url.clone(), None) {
        diagnostics::write(
            app,
            "browser_status",
            format!("{source} open failed url={url} error={error}"),
        );
    }
}

fn focus_status_window(window: &tauri::Window) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

fn inject_page_diagnostics(window: &tauri::Window, source: &str) {
    match window.eval(DIAGNOSTIC_EVAL_SCRIPT) {
        Ok(()) => diagnostics::write(
            &window.app_handle(),
            "diagnostics",
            format!("{source} eval injected successfully"),
        ),
        Err(error) => diagnostics::write(
            &window.app_handle(),
            "diagnostics",
            format!("{source} eval injection failed: {error}"),
        ),
    }
}

fn log_window_state(window: &tauri::Window, source: &str) {
    let inner_size = window
        .inner_size()
        .map(|size| format!("{}x{}", size.width, size.height))
        .unwrap_or_else(|error| format!("error:{error}"));
    let outer_size = window
        .outer_size()
        .map(|size| format!("{}x{}", size.width, size.height))
        .unwrap_or_else(|error| format!("error:{error}"));
    let scale_factor = window
        .scale_factor()
        .map(|factor| factor.to_string())
        .unwrap_or_else(|error| format!("error:{error}"));
    let visible = window
        .is_visible()
        .map(|value| value.to_string())
        .unwrap_or_else(|error| format!("error:{error}"));
    let focused = window
        .is_focused()
        .map(|value| value.to_string())
        .unwrap_or_else(|error| format!("error:{error}"));

    diagnostics::write(
        &window.app_handle(),
        "window",
        format!(
            "{source} label={} inner_size={} outer_size={} scale_factor={} visible={} focused={}",
            window.label(),
            inner_size,
            outer_size,
            scale_factor,
            visible,
            focused
        ),
    );
}

fn install_panic_hook() {
    panic::set_hook(Box::new(|panic_info| {
        diagnostics::bootstrap(format!("panic: {panic_info}"));
    }));
}

#[cfg(target_os = "windows")]
fn log_webview2_runtime_hint(app: &tauri::AppHandle) {
    use std::process::Command;
    use std::os::windows::process::CommandExt;

    let queries = [
        (
            "HKLM x64",
            r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        ),
        (
            "HKLM wow6432",
            r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        ),
        (
            "HKCU",
            r"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        ),
    ];

    for (label, key) in queries {
        let mut command = Command::new("reg");
        command.creation_flags(0x08000000);
        let output = command.args(["query", key, "/v", "pv"]).output();
        match output {
            Ok(output) if output.status.success() => diagnostics::write(
                app,
                "webview2",
                format!(
                    "{label} runtime={}",
                    String::from_utf8_lossy(&output.stdout).replace(['\r', '\n'], " ")
                ),
            ),
            Ok(output) => diagnostics::write(
                app,
                "webview2",
                format!(
                    "{label} runtime query failed status={} stderr={}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).replace(['\r', '\n'], " ")
                ),
            ),
            Err(error) => diagnostics::write(
                app,
                "webview2",
                format!("{label} runtime query failed: {error}"),
            ),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn log_webview2_runtime_hint(_app: &tauri::AppHandle) {}
