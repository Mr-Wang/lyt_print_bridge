#[cfg(target_os = "windows")]
use std::sync::mpsc;
#[cfg(target_os = "windows")]
use std::time::Duration;

use tauri::{AppHandle, Manager, Window, WindowBuilder, WindowUrl};

#[cfg(target_os = "windows")]
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_15_Vtbl;
#[cfg(target_os = "windows")]
use windows::core::Interface;

const PRINT_WINDOW_PREFIX: &str = "print-job-";

pub fn open_print_window(app: &AppHandle, job_id: &str, job_name: &str) -> Result<(), String> {
    let label = print_window_label(job_id);
    let target = format!("index.html#printHost=1&job={job_id}");
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
        window
            .eval(&format!(
                "window.location.replace({});",
                serde_json::to_string(&target).unwrap()
            ))
            .map_err(|error| format!("刷新打印窗口失败: {error}"))?;
        show_and_focus(&window);
        crate::diagnostics::write(
            app,
            "print_host",
            format!("existing visible print host refreshed label={label}"),
        );
        return Ok(());
    }

    let mut builder = WindowBuilder::new(app, label, WindowUrl::App(target.into()))
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
            crate::diagnostics::write(
                app,
                "print_host",
                format!("visible print host built label={}", window.label()),
            );
        })
        .map_err(|error| {
            crate::diagnostics::write(
                app,
                "print_window",
                format!(
                    "window build failed label={} error={}",
                    print_window_label(job_id),
                    error
                ),
            );
            format!("创建打印窗口失败: {error}")
        })
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

        let scheduling_window = window.clone();
        let callback_window = window.clone();
        let (tx, rx) = mpsc::sync_channel(1);

        scheduling_window
            .run_on_main_thread(move || {
                let inner_tx = tx.clone();
                let log_window = callback_window.clone();
                let with_webview_result = callback_window.with_webview(move |webview| unsafe {
                    crate::diagnostics::write(
                        &log_window.app_handle(),
                        "print_window",
                        format!(
                            "with_webview entered for print dialog label={}",
                            log_window.label()
                        ),
                    );
                    let controller = webview.controller();
                    let core = controller
                        .CoreWebView2()
                        .map_err(|error| format!("获取 WebView2 Core 失败: {error}"));

                    let result = match core {
                        Ok(core) => match core.cast::<ICoreWebView2_16>() {
                            Ok(core16) => core16
                                .show_print_ui(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)
                                .map_err(|error| {
                                    format!("调起 Windows 系统打印对话框失败: {error}")
                                }),
                            Err(error) => Err(format!(
                                "当前机器的 WebView2 Runtime 版本过低，不支持系统打印对话框: {error}"
                            )),
                        },
                        Err(error) => Err(error),
                    };

                    let _ = inner_tx.send(result);
                });

                if let Err(error) = with_webview_result {
                    crate::diagnostics::write(
                        &callback_window.app_handle(),
                        "print_window",
                        format!("with_webview failed label={} error={}", callback_window.label(), error),
                    );
                    let _ = tx.send(Err(format!("切换到打印 WebView 失败: {error}")));
                }
            })
            .map_err(|error| format!("切到主线程触发系统打印失败: {error}"))?;

        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "触发系统打印对话框超时，请重试。".to_string())?
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
