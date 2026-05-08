use std::{fs, path::PathBuf, sync::Arc, thread};

use base64::{engine::general_purpose::STANDARD, Engine};
#[cfg(target_os = "windows")]
use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use reqwest::blocking::Client;
use tauri::AppHandle;
#[cfg(target_os = "windows")]
use windows::Win32::{
    System::Memory::{GlobalFree, GlobalLock, GlobalUnlock},
    UI::Controls::Dialogs::{
        CommDlgExtendedError, PrintDlgW, DEVNAMES, PD_HIDEPRINTTOFILE, PD_NOSELECTION,
        PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW,
    },
};

use crate::app_state::{
    emit_snapshot, focus_main_window, generate_job_id, now_iso, BridgeCore, BridgeSettings,
    PrintJob, PrintJobStatus, PrintRequestPayload,
};

pub fn submit_http_print(
    app: &AppHandle,
    core: &Arc<BridgeCore>,
    request: PrintRequestPayload,
) -> Result<PrintJob, String> {
    let settings = core.settings();
    let normalized = normalize_request(request)?;
    let requested_at = now_iso();
    let job_id = generate_job_id();

    let initial_job = PrintJob {
        id: job_id.clone(),
        job_name: normalized.job_name.clone(),
        printer_name: normalized.printer_name.clone(),
        file_url: normalized.source_label.clone(),
        content_type: normalized.content_type.clone(),
        copies: normalized.copies,
        page_range: normalized.page_range.clone(),
        local_file_path: None,
        status: PrintJobStatus::Downloading,
        error: None,
        requested_at: requested_at.clone(),
        updated_at: requested_at,
    };
    core.insert_job(initial_job);
    emit_snapshot(app, core);

    let local_file = match materialize_pdf(&settings, &job_id, &normalized) {
        Ok(local_file) => local_file,
        Err(error) => {
            core.update_job(&job_id, |job| {
                job.status = PrintJobStatus::Failed;
                job.error = Some(error.clone());
            })?;
            emit_snapshot(app, core);
            return Err(error);
        }
    };
    let next_status = if settings.confirmation_required {
        PrintJobStatus::PendingConfirmation
    } else {
        PrintJobStatus::Printing
    };

    let queued_job = core.update_job(&job_id, |job| {
        job.local_file_path = Some(local_file.to_string_lossy().into_owned());
        job.status = next_status.clone();
        job.error = None;
    })?;
    emit_snapshot(app, core);

    if settings.confirmation_required {
        focus_main_window(app);
        return Ok(queued_job);
    }

    approve_job(app, core, &job_id)?;
    core.find_job(&job_id)
        .ok_or_else(|| format!("自动提交后未找到打印任务: {job_id}"))
}

pub fn approve_job(app: &AppHandle, core: &Arc<BridgeCore>, job_id: &str) -> Result<(), String> {
    crate::diagnostics::write(app, "print_service", format!("approve_job job_id={job_id}"));
    let job = core
        .find_job(job_id)
        .ok_or_else(|| format!("未找到打印任务: {job_id}"))?;

    if matches!(job.status, PrintJobStatus::Cancelled) {
        return Err("任务已经取消，不能继续打印".into());
    }

    core.update_job(job_id, |target| {
        target.status = PrintJobStatus::Printing;
        target.error = None;
    })?;
    emit_snapshot(app, core);

    let file_path = job
        .local_file_path
        .clone()
        .ok_or_else(|| format!("打印任务缺少本地 PDF 文件: {job_id}"))?;

    crate::diagnostics::write(
        app,
        "print_service",
        format!(
            "approve_job opening print window job_id={} file_path={} job_name={}",
            job_id, file_path, job.job_name
        ),
    );

    if !PathBuf::from(&file_path).exists() {
        return Err(format!("本地 PDF 文件不存在: {file_path}"));
    }

    #[cfg(target_os = "windows")]
    {
        return approve_job_with_windows_shell_print(app, core, job_id, job, file_path);
    }

    #[cfg(not(target_os = "windows"))]
    {
        approve_job_with_webview_print_window(app, core, job_id, job, file_path)
    }
}

#[cfg(target_os = "windows")]
fn approve_job_with_windows_shell_print(
    app: &AppHandle,
    core: &Arc<BridgeCore>,
    job_id: &str,
    job: PrintJob,
    file_path: String,
) -> Result<(), String> {
    let app_for_print = app.clone();
    let core_for_print = Arc::clone(core);
    let thread_job_id = job_id.to_string();

    thread::Builder::new()
        .name(format!("shell-print-{job_id}"))
        .spawn(move || {
            crate::diagnostics::write(
                &app_for_print,
                "print_service",
                format!(
                    "background shell print task started job_id={} file_path={} job_name={}",
                    thread_job_id, file_path, job.job_name
                ),
            );

            let result = show_windows_print_dialog_and_dispatch(&app_for_print, &file_path);
            match result {
                Ok(Some(printer_name)) => {
                    crate::diagnostics::write(
                        &app_for_print,
                        "print_service",
                        format!(
                            "background shell print task dispatched job_id={} printer={}",
                            thread_job_id, printer_name
                        ),
                    );
                    let _ = core_for_print.update_job(&thread_job_id, |target| {
                        target.status = PrintJobStatus::DialogOpened;
                        target.error =
                            Some(format!("已选择打印机并通过 Windows PrintTo 动作处理: {printer_name}"));
                    });
                    emit_snapshot(&app_for_print, &core_for_print);
                }
                Ok(None) => {
                    crate::diagnostics::write(
                        &app_for_print,
                        "print_service",
                        format!("native print dialog cancelled job_id={thread_job_id}"),
                    );
                    let _ = core_for_print.update_job(&thread_job_id, |target| {
                        target.status = PrintJobStatus::Cancelled;
                        target.error = None;
                    });
                    emit_snapshot(&app_for_print, &core_for_print);
                }
                Err(error) => {
                    crate::diagnostics::write(
                        &app_for_print,
                        "print_service",
                        format!(
                            "background shell print task failed job_id={} error={}",
                            thread_job_id, error
                        ),
                    );
                    let _ = core_for_print.update_job(&thread_job_id, |target| {
                        target.status = PrintJobStatus::Failed;
                        target.error = Some(error.clone());
                    });
                    emit_snapshot(&app_for_print, &core_for_print);
                }
            }
        })
        .map_err(|error| format!("启动系统打印线程失败: {error}"))?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn show_windows_print_dialog_and_dispatch(
    app: &AppHandle,
    file_path: &str,
) -> Result<Option<String>, String> {
    let detected = detect_pdf_page_count(file_path);
    crate::diagnostics::write(
        app,
        "print_service",
        format!(
            "detected PDF page_count={} method={}{} file_path={}",
            detected.count,
            detected.method,
            detected
                .note
                .as_ref()
                .map(|note| format!(" note={note}"))
                .unwrap_or_default(),
            file_path
        ),
    );
    let printer_name = show_windows_print_dialog(detected.count)?;
    match printer_name {
        Some(printer_name) => {
            open_windows_shell_print_to(file_path, &printer_name)?;
            Ok(Some(printer_name))
        }
        None => Ok(None),
    }
}

#[cfg(target_os = "windows")]
fn show_windows_print_dialog(page_count: u16) -> Result<Option<String>, String> {
    let page_count = page_count.max(1);
    let mut dialog = PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        Flags: PD_RETURNDC
            | PD_USEDEVMODECOPIESANDCOLLATE
            | PD_HIDEPRINTTOFILE
            | PD_NOSELECTION,
        nMinPage: 1,
        nMaxPage: page_count,
        nFromPage: 1,
        nToPage: page_count,
        nCopies: 1,
        ..Default::default()
    };

    let accepted = unsafe { PrintDlgW(&mut dialog).as_bool() };
    if !accepted {
        let error = unsafe { CommDlgExtendedError() };
        if error.0 == 0 {
            return Ok(None);
        }
        return Err(format!("Windows 打印对话框打开失败，错误码: {}", error.0));
    }

    let printer_name = unsafe { read_printer_name_from_devnames(dialog.hDevNames) }
        .unwrap_or_else(|| "默认打印机".to_string());

    unsafe {
        if dialog.hDevMode != 0 {
            let _ = GlobalFree(dialog.hDevMode);
        }
        if dialog.hDevNames != 0 {
            let _ = GlobalFree(dialog.hDevNames);
        }
    }

    Ok(Some(printer_name))
}

#[cfg(target_os = "windows")]
struct PdfPageCountDetection {
    count: u16,
    method: &'static str,
    note: Option<String>,
}

#[cfg(target_os = "windows")]
fn detect_pdf_page_count(file_path: &str) -> PdfPageCountDetection {
    match detect_pdf_page_count_with_lopdf(file_path) {
        Ok(count) => PdfPageCountDetection {
            count,
            method: "lopdf",
            note: None,
        },
        Err(parser_error) => match fs::read(file_path) {
            Ok(bytes) => PdfPageCountDetection {
                count: bounded_pdf_page_count(count_pdf_page_markers(&bytes)),
                method: "marker-fallback",
                note: Some(format!("parser_error={parser_error}")),
            },
            Err(read_error) => PdfPageCountDetection {
                count: 1,
                method: "default",
                note: Some(format!(
                    "parser_error={parser_error}; read_error={read_error}"
                )),
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn detect_pdf_page_count_with_lopdf(file_path: &str) -> Result<u16, String> {
    let document = lopdf::Document::load(file_path)
        .map_err(|error| format!("lopdf_load_failed:{error}"))?;
    let count = document.get_pages().len();
    if count == 0 {
        return Err("lopdf_returned_zero_pages".into());
    }
    Ok(bounded_pdf_page_count(count))
}

#[cfg(target_os = "windows")]
fn bounded_pdf_page_count(count: usize) -> u16 {
    count.clamp(1, u16::MAX as usize) as u16
}

#[cfg(target_os = "windows")]
fn count_pdf_page_markers(bytes: &[u8]) -> usize {
    let mut count = 0usize;
    let mut index = 0usize;

    while let Some(relative) = find_subslice(&bytes[index..], b"/Type") {
        let type_index = index + relative + b"/Type".len();
        let after_type = skip_pdf_whitespace(bytes, type_index);
        if bytes
            .get(after_type..after_type + b"/Page".len())
            .map(|value| value == b"/Page")
            .unwrap_or(false)
            && !matches!(bytes.get(after_type + b"/Page".len()), Some(b's'))
        {
            count += 1;
        }
        index = type_index;
    }

    count
}

#[cfg(target_os = "windows")]
fn skip_pdf_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while matches!(
        bytes.get(index),
        Some(b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
    ) {
        index += 1;
    }
    index
}

#[cfg(target_os = "windows")]
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(target_os = "windows")]
unsafe fn read_printer_name_from_devnames(handle: isize) -> Option<String> {
    if handle == 0 {
        return None;
    }

    let locked = GlobalLock(handle);
    if locked.is_null() {
        return None;
    }

    let devnames = *(locked as *const DEVNAMES);
    let base = locked as *const u16;
    let name_ptr = base.add(devnames.wDeviceOffset as usize);
    let mut len = 0usize;
    while *name_ptr.add(len) != 0 {
        len += 1;
    }
    let value = String::from_utf16_lossy(std::slice::from_raw_parts(name_ptr, len));
    let _ = GlobalUnlock(handle);
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(target_os = "windows")]
fn open_windows_shell_print_to(file_path: &str, printer_name: &str) -> Result<(), String> {
    let script = "Start-Process -FilePath $args[0] -Verb PrintTo -ArgumentList ('\"{0}\"' -f $args[1])";
    let mut command = hidden_command("powershell");
    command
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
            "--",
            file_path,
            printer_name,
        ]);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("调用 Windows Shell PrintTo 动作失败: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn approve_job_with_webview_print_window(
    app: &AppHandle,
    core: &Arc<BridgeCore>,
    job_id: &str,
    job: PrintJob,
    _file_path: String,
) -> Result<(), String> {
    let app_for_window = app.clone();
    let core_for_window = Arc::clone(core);
    let job_for_window = job.clone();
    let thread_job_id = job_id.to_string();

    thread::Builder::new()
        .name(format!("print-window-{job_id}"))
        .spawn(move || {
            crate::diagnostics::write(
                &app_for_window,
                "print_service",
                format!("background print window task started job_id={thread_job_id}"),
            );

            match crate::platform_print::open_print_window(
                &app_for_window,
                &job_for_window.id,
                &job_for_window.job_name,
            ) {
                Ok(()) => {
                    crate::diagnostics::write(
                        &app_for_window,
                        "print_service",
                        format!("background print window task finished job_id={thread_job_id}"),
                    );
                    emit_snapshot(&app_for_window, &core_for_window);
                }
                Err(error) => {
                    crate::diagnostics::write(
                        &app_for_window,
                        "print_service",
                        format!(
                            "background print window task failed job_id={} error={}",
                            thread_job_id, error
                        ),
                    );
                    let _ = core_for_window.update_job(&thread_job_id, |target| {
                        target.status = PrintJobStatus::Failed;
                        target.error = Some(error.clone());
                    });
                    emit_snapshot(&app_for_window, &core_for_window);
                }
            }
        })
        .map_err(|error| format!("启动打印窗口线程失败: {error}"))?;

    Ok(())
}

pub fn cancel_job(app: &AppHandle, core: &Arc<BridgeCore>, job_id: &str) -> Result<(), String> {
    let settings = core.settings();
    let current = core
        .find_job(job_id)
        .ok_or_else(|| format!("未找到打印任务: {job_id}"))?;
    if matches!(current.status, PrintJobStatus::Completed) {
        return Err("已完成任务不能取消".into());
    }

    core.update_job(job_id, |job| {
        job.status = PrintJobStatus::Cancelled;
        job.error = None;
    })?;

    if !settings.keep_downloaded_files {
        if let Some(path) = current.local_file_path {
            let _ = fs::remove_file(path);
        }
    }

    emit_snapshot(app, core);
    Ok(())
}

pub fn list_printers() -> Result<Vec<String>, String> {
    #[cfg(target_os = "windows")]
    {
        let mut errors = Vec::new();

        for (program, args, parser) in windows_printer_queries() {
            match run_windows_text_command(program, args) {
                Ok(output) => {
                    let printers = parser(&output);
                    if !printers.is_empty() {
                        return Ok(printers);
                    }
                    errors.push(format!("{program} 未返回打印机名称"));
                }
                Err(error) => errors.push(format!("{program}: {error}")),
            }
        }

        return Err(format!(
            "未能读取本机打印机列表。请确认 Windows 打印服务正常，或把错误信息发我：{}",
            errors.join(" | ")
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let mut errors = Vec::new();

        for (program, args) in linux_printer_queries() {
            match run_linux_text_command(program, args) {
                Ok(output) => {
                    let printers = parse_printer_lines(&output);
                    if !printers.is_empty() {
                        return Ok(printers);
                    }
                    errors.push(format!("{program} 未返回打印机名称"));
                }
                Err(error) => errors.push(format!("{program}: {error}")),
            }
        }

        return Err(format!(
            "未能读取本机打印机列表。请确认 CUPS 打印服务正常，或把错误信息发我：{}",
            errors.join(" | ")
        ));
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Ok(Vec::new())
    }
}

pub fn detect_sumatra_pdf_path(configured_path: &str) -> Option<String> {
    let _ = configured_path;
    None
}

fn normalize_request(request: PrintRequestPayload) -> Result<NormalizedPrintRequest, String> {
    let content_type = request
        .content_type
        .unwrap_or_else(|| "pdf".into())
        .trim()
        .to_ascii_lowercase();
    if content_type != "pdf" {
        return Err("当前只支持 PDF 打印链路".into());
    }

    let remote_url = request.file_url.and_then(trimmed_option);
    let local_base64 = request.file_base64.and_then(trimmed_option);

    if remote_url.is_some() == local_base64.is_some() {
        return Err("必须且只能提供一种 PDF 来源：fileUrl 或 fileBase64".into());
    }

    let copies = request.copies.unwrap_or(1).max(1);
    let page_range = request.page_range.and_then(trimmed_option);
    let printer_name = request.printer_name.and_then(trimmed_option);
    let job_name = request
        .job_name
        .and_then(trimmed_option)
        .unwrap_or_else(|| "PDF 打印任务".into());
    let file_name = request
        .file_name
        .and_then(trimmed_option)
        .unwrap_or_else(|| format!("{}.pdf", sanitize_file_name(&job_name)));

    let source = match (&remote_url, &local_base64) {
        (Some(file_url), None) => {
            if !(file_url.starts_with("http://") || file_url.starts_with("https://")) {
                return Err("fileUrl 必须是 http 或 https 地址".into());
            }
            PdfSource::RemoteUrl(file_url.clone())
        }
        (None, Some(file_base64)) => PdfSource::Base64Upload {
            file_name: file_name.clone(),
            file_base64: file_base64.clone(),
        },
        _ => return Err("未识别的 PDF 来源参数".into()),
    };

    Ok(NormalizedPrintRequest {
        printer_name,
        job_name,
        source_label: source.source_label(),
        source,
        file_name,
        content_type,
        copies,
        page_range,
    })
}

fn materialize_pdf(
    settings: &BridgeSettings,
    job_id: &str,
    request: &NormalizedPrintRequest,
) -> Result<PathBuf, String> {
    let download_dir = PathBuf::from(&settings.download_dir);
    fs::create_dir_all(&download_dir).map_err(|error| format!("创建下载目录失败: {error}"))?;

    let file_name = format!("{job_id}-{}.pdf", sanitize_file_name(&request.file_name));
    let target_path = download_dir.join(file_name);

    match &request.source {
        PdfSource::RemoteUrl(file_url) => {
            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .build()
                .map_err(|error| format!("创建下载客户端失败: {error}"))?;
            let response = client
                .get(file_url)
                .send()
                .map_err(|error| format!("下载 PDF 失败: {error}"))?;
            if !response.status().is_success() {
                return Err(format!("下载文件失败，HTTP {}", response.status()));
            }

            let bytes = response
                .bytes()
                .map_err(|error| format!("读取 PDF 响应体失败: {error}"))?;
            validate_pdf_bytes(bytes.as_ref())?;
            fs::write(&target_path, &bytes)
                .map_err(|error| format!("写入下载文件失败: {error}"))?;
        }
        PdfSource::Base64Upload { file_base64, .. } => {
            let bytes = STANDARD
                .decode(file_base64)
                .map_err(|error| format!("解析本地 PDF base64 数据失败: {error}"))?;
            validate_pdf_bytes(&bytes)?;
            fs::write(&target_path, &bytes)
                .map_err(|error| format!("写入本地 PDF 文件失败: {error}"))?;
        }
    }

    Ok(target_path)
}

#[cfg(target_os = "windows")]
type PrinterQueryParser = fn(&str) -> Vec<String>;

#[cfg(target_os = "windows")]
fn windows_printer_queries() -> Vec<(&'static str, &'static [&'static str], PrinterQueryParser)> {
    vec![
        (
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-Printer | Select-Object -ExpandProperty Name",
            ],
            parse_printer_lines,
        ),
        (
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-CimInstance Win32_Printer | Select-Object -ExpandProperty Name",
            ],
            parse_printer_lines,
        ),
        (
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [System.Drawing.Printing.PrinterSettings]::InstalledPrinters",
            ],
            parse_printer_lines,
        ),
        (
            "wmic",
            &["printer", "get", "name"],
            parse_printer_lines,
        ),
    ]
}

#[cfg(target_os = "windows")]
fn run_windows_text_command(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = hidden_command(program);
    let output = command
        .args(args)
        .output()
        .map_err(|error| format!("执行失败: {error}"))?;

    if !output.status.success() {
        let stderr = decode_windows_output(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("退出码 {}", output.status)
        } else {
            stderr.trim().to_string()
        });
    }

    Ok(decode_windows_output(&output.stdout))
}

#[cfg(target_os = "windows")]
fn hidden_command(program: &str) -> Command {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
fn decode_windows_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let looks_like_utf16 = bytes.len() >= 2
        && ((bytes[0] == 0xFF && bytes[1] == 0xFE)
            || bytes
                .iter()
                .skip(1)
                .step_by(2)
                .filter(|byte| **byte == 0)
                .count()
                * 3
                > bytes.len());

    if looks_like_utf16 {
        let mut units = Vec::new();
        for chunk in bytes.chunks_exact(2) {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        return String::from_utf16_lossy(&units)
            .trim_matches('\u{feff}')
            .to_string();
    }

    String::from_utf8_lossy(bytes)
        .trim_matches('\u{feff}')
        .to_string()
}

#[cfg(target_os = "linux")]
fn linux_printer_queries() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("lpstat", &["-e"]),
        ("lpstat", &["-p"]),
    ]
}

#[cfg(target_os = "linux")]
fn run_linux_text_command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("执行失败: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("退出码 {}", output.status)
        } else {
            stderr.trim().to_string()
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn parse_printer_lines(output: &str) -> Vec<String> {
    let mut printers = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim().trim_matches('\u{feff}');
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("name") {
            continue;
        }
        let trimmed = trimmed
            .strip_prefix("printer ")
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or(trimmed);
        if !printers.iter().any(|value| value == trimmed) {
            printers.push(trimmed.to_string());
        }
    }

    printers
}

fn sanitize_file_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "print-job".into()
    } else {
        trimmed.into()
    }
}

fn trimmed_option(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Clone)]
struct NormalizedPrintRequest {
    printer_name: Option<String>,
    job_name: String,
    source_label: String,
    source: PdfSource,
    file_name: String,
    content_type: String,
    copies: u32,
    page_range: Option<String>,
}

#[derive(Clone)]
enum PdfSource {
    RemoteUrl(String),
    Base64Upload {
        file_name: String,
        file_base64: String,
    },
}

impl PdfSource {
    fn source_label(&self) -> String {
        match self {
            Self::RemoteUrl(file_url) => file_url.clone(),
            Self::Base64Upload { file_name, .. } => format!("local-upload://{file_name}"),
        }
    }
}

fn validate_pdf_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.starts_with(b"%PDF") {
        Ok(())
    } else {
        Err("提供的文件内容不是合法 PDF".into())
    }
}
