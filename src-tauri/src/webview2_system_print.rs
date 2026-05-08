use std::time::Duration;

use crate::app_state::{FIXED_ACCESS_TOKEN, FIXED_PORT};
use tauri::{AppHandle, Manager, Window, WindowBuilder, WindowUrl};
#[cfg(target_os = "windows")]
use webview2_com::CoTaskMemPWSTR;

#[cfg(target_os = "windows")]
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_15_Vtbl;
#[cfg(target_os = "windows")]
use windows::core::Interface;

const PRINT_WINDOW_PREFIX: &str = "print-job-";

pub fn open_print_window(app: &AppHandle, job_id: &str, job_name: &str) -> Result<(), String> {
    let label = print_window_label(job_id);
    let target = format!(
        "http://127.0.0.1:{FIXED_PORT}/printer/jobs/{job_id}/document?token={FIXED_ACCESS_TOKEN}"
    );
    crate::diagnostics::write(
        app,
        "print_window",
        format!(
            "open requested label={} target={} job_id={} job_name={}",
            label, target, job_id, job_name
        ),
    );

    if let Some(window) = app.get_window(&label) {
        crate::diagnostics::write(
            app,
            "print_window",
            format!("reuse existing window label={} target={}", label, target),
        );
        native_navigate(&window, &target)?;
        show_and_focus(&window);
        schedule_print_dialog(window.clone(), job_id.to_string());
        crate::diagnostics::write(
            app,
            "print_host",
            format!("existing print host navigated to local document label={label}"),
        );
        return Ok(());
    }

    let print_url = target
        .parse()
        .map_err(|error| format!("解析打印窗口地址失败: {error}"))?;
    let mut builder = WindowBuilder::new(app, label.clone(), WindowUrl::External(print_url))
        .title(&format!("打印任务 - {job_name}"))
        .resizable(false)
        .inner_size(980.0, 780.0);

    #[cfg(target_os = "windows")]
    {
        builder = builder.visible(true).focused(true).skip_taskbar(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        builder = builder.visible(false).focused(false).skip_taskbar(true);
    }

    builder
        .build()
        .map(|window| {
            show_and_focus(&window);
            if let Err(error) = native_navigate(&window, &target) {
                crate::diagnostics::write(
                    app,
                    "print_window",
                    format!(
                        "native navigate failed after build label={} error={}",
                        window.label(),
                        error
                    ),
                );
            }
            schedule_print_dialog(window.clone(), job_id.to_string());
            crate::diagnostics::write(
                app,
                "print_host",
                format!("visible print host built with local document label={}", window.label()),
            );
        })
        .map_err(|error| {
            crate::diagnostics::write(
                app,
                "print_window",
                format!(
                    "window build failed label={} error={}",
                    label,
                    error
                ),
            );
            format!("创建打印窗口失败: {error}")
        })
}

fn native_navigate(window: &Window, target: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        native_navigate_windows(window, target)
    }

    #[cfg(not(target_os = "windows"))]
    {
        window
            .eval(&format!(
                "window.location.replace({});",
                serde_json::to_string(target).unwrap()
            ))
            .map_err(|error| format!("刷新打印窗口失败: {error}"))
    }
}

#[cfg(target_os = "windows")]
fn native_navigate_windows(window: &Window, target: &str) -> Result<(), String> {
    let app = window.app_handle();
    let label = window.label().to_string();
    let target = target.to_string();
    window
        .with_webview(move |webview| unsafe {
            crate::diagnostics::write(
                &app,
                "print_window",
                format!("native navigate entered label={label} target={target}"),
            );
            let controller = webview.controller();
            match controller
                .CoreWebView2()
                .map_err(|error| format!("获取 WebView2 Core 失败: {error}"))
                .and_then(|core| {
                    let url = CoTaskMemPWSTR::from(target.as_str());
                    core.Navigate(*url.as_ref().as_pcwstr())
                        .map_err(|error| format!("WebView2 Navigate 失败: {error}"))
                }) {
                Ok(()) => crate::diagnostics::write(
                    &app,
                    "print_window",
                    format!("native navigate dispatched label={label}"),
                ),
                Err(error) => crate::diagnostics::write(
                    &app,
                    "print_window",
                    format!("native navigate failed label={} error={}", label, error),
                ),
            }
        })
        .map_err(|error| format!("切换到打印 WebView 导航失败: {error}"))
}

fn schedule_print_dialog(window: Window, job_id: String) {
    let label = window.label().to_string();
    let label_for_error = label.clone();
    let app = window.app_handle();
    let app_for_error = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(format!("print-dialog-{job_id}"))
        .spawn(move || {
            crate::diagnostics::write(
                &app,
                "print_window",
                format!("scheduled print dialog waiting label={label} job_id={job_id}"),
            );
            std::thread::sleep(Duration::from_millis(3500));
            match trigger_system_print_dialog(&window) {
                Ok(()) => crate::diagnostics::write(
                    &app,
                    "print_window",
                    format!("scheduled print dialog triggered label={label} job_id={job_id}"),
                ),
                Err(error) => crate::diagnostics::write(
                    &app,
                    "print_window",
                    format!(
                        "scheduled print dialog failed label={} job_id={} error={}",
                        label, job_id, error
                    ),
                ),
            }
        }) {
        crate::diagnostics::write(
            &app_for_error,
            "print_window",
            format!("failed to spawn print dialog scheduler label={label_for_error}: {error}"),
        );
    }
}

pub fn trigger_system_print_dialog(window: &Window) -> Result<(), String> {
    crate::diagnostics::write(
        &window.app_handle(),
        "print_window",
        format!("trigger_system_print_dialog label={}", window.label()),
    );
    show_and_focus(window);

    #[cfg(target_os = "windows")]
    {
        std::thread::sleep(Duration::from_millis(250));

        let callback_window = window.clone();
        let log_window = callback_window.clone();
        callback_window
            .with_webview(move |webview| unsafe {
                crate::diagnostics::write(
                    &log_window.app_handle(),
                    "print_window",
                    format!(
                        "with_webview entered for print dialog label={}",
                        log_window.label()
                    ),
                );
                let controller = webview.controller();
                match controller
                    .CoreWebView2()
                    .map_err(|error| format!("获取 WebView2 Core 失败: {error}"))
                    .and_then(|core| {
                        core.cast::<ICoreWebView2_16>().map_err(|error| {
                            format!(
                                "当前机器的 WebView2 Runtime 版本过低，不支持系统打印对话框: {error}"
                            )
                        })
                    })
                    .and_then(|core16| {
                        core16
                            .show_print_ui(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)
                            .map_err(|error| format!("调起 Windows 系统打印对话框失败: {error}"))
                    }) {
                    Ok(()) => crate::diagnostics::write(
                        &log_window.app_handle(),
                        "print_window",
                        format!("show_print_ui dispatched label={}", log_window.label()),
                    ),
                    Err(error) => crate::diagnostics::write(
                        &log_window.app_handle(),
                        "print_window",
                        format!(
                            "show_print_ui failed label={} error={}",
                            log_window.label(),
                            error
                        ),
                    ),
                }
            })
            .map_err(|error| format!("切换到打印 WebView 失败: {error}"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        show_and_focus(window);
        window
            .eval("window.print()")
            .map_err(|error| format!("触发系统打印对话框失败: {error}"))
    }
}

pub fn print_window_label(job_id: &str) -> String {
    format!("{PRINT_WINDOW_PREFIX}{job_id}")
}

fn show_and_focus(window: &Window) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

#[cfg(target_os = "windows")]
#[repr(transparent)]
struct ICoreWebView2_16(::windows::core::IUnknown);

#[cfg(target_os = "windows")]
impl ICoreWebView2_16 {
    fn show_print_ui(
        &self,
        print_dialog_kind: COREWEBVIEW2_PRINT_DIALOG_KIND,
    ) -> ::windows::core::Result<()> {
        unsafe {
            (::windows::core::Interface::vtable(self).show_print_ui)(
                ::windows::core::Interface::as_raw(self),
                print_dialog_kind,
            )
            .ok()
        }
    }
}

#[cfg(target_os = "windows")]
impl Clone for ICoreWebView2_16 {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg(target_os = "windows")]
unsafe impl ::windows::core::Interface for ICoreWebView2_16 {
    type Vtable = ICoreWebView2_16_Vtbl;
    const IID: ::windows::core::GUID =
        ::windows::core::GUID::from_u128(0x0eb34dc9_9f91_41e1_8639_95cd5943906b);
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[allow(non_camel_case_types, non_snake_case)]
struct ICoreWebView2_16_Vtbl {
    pub base__: ICoreWebView2_15_Vtbl,
    pub Print: unsafe extern "system" fn(
        this: *mut ::core::ffi::c_void,
        printsettings: *mut ::core::ffi::c_void,
        handler: *mut ::core::ffi::c_void,
    ) -> ::windows::core::HRESULT,
    pub show_print_ui: unsafe extern "system" fn(
        this: *mut ::core::ffi::c_void,
        printdialogkind: COREWEBVIEW2_PRINT_DIALOG_KIND,
    ) -> ::windows::core::HRESULT,
    pub PrintToPdfStream: unsafe extern "system" fn(
        this: *mut ::core::ffi::c_void,
        printsettings: *mut ::core::ffi::c_void,
        handler: *mut ::core::ffi::c_void,
    ) -> ::windows::core::HRESULT,
}

#[cfg(target_os = "windows")]
#[repr(transparent)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct COREWEBVIEW2_PRINT_DIALOG_KIND(pub i32);

#[cfg(target_os = "windows")]
const COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM: COREWEBVIEW2_PRINT_DIALOG_KIND =
    COREWEBVIEW2_PRINT_DIALOG_KIND(1);
