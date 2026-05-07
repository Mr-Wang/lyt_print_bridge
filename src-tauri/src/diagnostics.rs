use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use tauri::{api::path, AppHandle};

const LOG_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "print-bridge.log";
const DESKTOP_LOG_DIR_NAME: &str = "liaoyitong-print-bridge-logs";

pub fn bootstrap(message: impl AsRef<str>) {
    let line = format!(
        "[{}] [bootstrap] {}\n",
        chrono::Utc::now().to_rfc3339(),
        normalize_line(message.as_ref())
    );

    for target in bootstrap_targets() {
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(target) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn log_dir(app: &AppHandle) -> PathBuf {
    path::app_config_dir(&app.config())
        .unwrap_or_else(|| std::env::temp_dir().join("lyt_print_bridge"))
        .join(LOG_DIR_NAME)
}

pub fn log_file(app: &AppHandle) -> PathBuf {
    log_dir(app).join(LOG_FILE_NAME)
}

pub fn desktop_log_dir() -> Option<PathBuf> {
    path::desktop_dir().map(|dir| dir.join(DESKTOP_LOG_DIR_NAME))
}

pub fn desktop_log_file() -> Option<PathBuf> {
    desktop_log_dir().map(|dir| dir.join(LOG_FILE_NAME))
}

fn bootstrap_targets() -> Vec<PathBuf> {
    let mut targets = vec![std::env::temp_dir()
        .join("liaoyitong-print-bridge-logs")
        .join(LOG_FILE_NAME)];

    if let Some(desktop_log_file) = desktop_log_file() {
        targets.push(desktop_log_file);
    }

    #[cfg(target_os = "windows")]
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        targets.push(
            PathBuf::from(user_profile)
                .join("Desktop")
                .join(DESKTOP_LOG_DIR_NAME)
                .join(LOG_FILE_NAME),
        );
    }

    targets
}

pub fn ensure_log_file(app: &AppHandle) -> Result<PathBuf, String> {
    try_write(app, "diagnostics", "log file initialized")?;
    Ok(log_file(app))
}

pub fn write(app: &AppHandle, source: impl AsRef<str>, message: impl AsRef<str>) {
    if let Err(error) = try_write(app, source, message) {
        eprintln!("failed to write print bridge log: {error}");
    }
}

pub fn try_write(
    app: &AppHandle,
    source: impl AsRef<str>,
    message: impl AsRef<str>,
) -> Result<(), String> {
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_string();
    let message = normalize_line(message.as_ref());

    let line = format!(
        "[{}] [{}] [{}] {}",
        chrono::Utc::now().to_rfc3339(),
        normalize_line(source.as_ref()),
        thread,
        message
    );

    let mut targets = vec![log_file(app)];
    if let Some(desktop_log_file) = desktop_log_file() {
        targets.push(desktop_log_file);
    }

    let mut errors = Vec::new();
    let mut wrote_any = false;

    for target in targets {
        match append_line(&target, &line) {
            Ok(()) => wrote_any = true,
            Err(error) => errors.push(format!("{}: {}", target.display(), error)),
        }
    }

    if wrote_any {
        Ok(())
    } else {
        Err(format!("写入日志失败: {}", errors.join(" | ")))
    }
}

fn append_line(path: &PathBuf, line: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "日志路径缺少父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建日志目录失败: {error}"))?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("打开日志文件失败: {error}"))?;

    writeln!(file, "{line}").map_err(|error| format!("写入日志失败: {error}"))
}

fn normalize_line(input: &str) -> String {
    input.replace('\r', "\\r").replace('\n', "\\n")
}
