use std::path::PathBuf;

pub fn ensure_enabled() -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|error| format!("获取程序路径失败: {error}"))?;
    ensure_platform_autostart(exe_path)
}

#[cfg(target_os = "windows")]
fn ensure_platform_autostart(exe_path: PathBuf) -> Result<(), String> {
    let exe = exe_path.to_string_lossy();
    let quoted = format!("\"{exe}\"");
    let status = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "LiaoyitongPrintBridge",
            "/t",
            "REG_SZ",
            "/d",
            &quoted,
            "/f",
        ])
        .status()
        .map_err(|error| format!("写入开机自启注册表失败: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("写入开机自启注册表失败，退出码: {status}"))
    }
}

#[cfg(target_os = "linux")]
fn ensure_platform_autostart(exe_path: PathBuf) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "未找到 HOME 目录".to_string())?;
    let autostart_dir = PathBuf::from(home).join(".config").join("autostart");
    std::fs::create_dir_all(&autostart_dir)
        .map_err(|error| format!("创建开机自启目录失败: {error}"))?;

    let desktop_file = autostart_dir.join("liaoyitong-print-bridge.desktop");
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=Liaoyitong Print Bridge\nExec=\"{}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe_path.to_string_lossy().replace('"', "\\\"")
    );
    std::fs::write(desktop_file, content).map_err(|error| format!("写入开机自启文件失败: {error}"))
}

#[cfg(target_os = "macos")]
fn ensure_platform_autostart(_exe_path: PathBuf) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn ensure_platform_autostart(_exe_path: PathBuf) -> Result<(), String> {
    Ok(())
}
