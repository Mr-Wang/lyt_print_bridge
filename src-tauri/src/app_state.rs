use chrono::Utc;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{api::path, AppHandle, Manager};

const SETTINGS_FILE_NAME: &str = "bridge-settings.json";
const SETTINGS_VERSION: u32 = 2;
const MAX_RECENT_JOBS: usize = 40;
const SNAPSHOT_EVENT: &str = "print-bridge://snapshot";
pub const FIXED_PORT: u16 = 12734;
pub const FIXED_ACCESS_TOKEN: &str = "liaoyitong-print-bridge-token-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeSettings {
    pub port: u16,
    pub access_token: String,
    pub allowed_origins: Vec<String>,
    pub sumatra_pdf_path: String,
    pub download_dir: String,
    pub keep_downloaded_files: bool,
    pub confirmation_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrintJobStatus {
    Downloading,
    PendingConfirmation,
    Printing,
    DialogOpened,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintJob {
    pub id: String,
    pub job_name: String,
    pub printer_name: Option<String>,
    pub file_url: String,
    pub content_type: String,
    pub copies: u32,
    pub page_range: Option<String>,
    pub local_file_path: Option<String>,
    pub status: PrintJobStatus,
    pub error: Option<String>,
    pub requested_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintRequestPayload {
    pub printer_name: Option<String>,
    pub job_name: Option<String>,
    pub file_url: Option<String>,
    pub file_name: Option<String>,
    pub file_base64: Option<String>,
    pub content_type: Option<String>,
    pub copies: Option<u32>,
    pub page_range: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsInput {
    pub port: u16,
    pub allowed_origins: Vec<String>,
    pub sumatra_pdf_path: String,
    pub download_dir: String,
    pub keep_downloaded_files: bool,
    pub confirmation_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub running: bool,
    pub port: u16,
    pub base_url: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub settings: BridgeSettings,
    pub service: ServiceStatus,
    pub jobs: Vec<PrintJob>,
    pub config_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintJobDocument {
    pub id: String,
    pub job_name: String,
    pub local_file_path: String,
    pub printer_name: Option<String>,
    pub copies: u32,
    pub page_range: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    version: u32,
    settings: BridgeSettings,
}

pub struct BridgeCore {
    settings: Mutex<BridgeSettings>,
    jobs: Mutex<VecDeque<PrintJob>>,
    service_running: Mutex<bool>,
    last_error: Mutex<Option<String>>,
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<BridgeCore>,
}

impl BridgeSettings {
    fn default_for_dir(config_dir: &Path) -> Self {
        Self {
            port: FIXED_PORT,
            access_token: FIXED_ACCESS_TOKEN.to_string(),
            allowed_origins: vec!["*".into()],
            sumatra_pdf_path: String::new(),
            download_dir: config_dir.join("downloads").to_string_lossy().into_owned(),
            keep_downloaded_files: false,
            confirmation_required: false,
        }
    }

    fn normalize(mut self, config_dir: &Path) -> Self {
        self.port = FIXED_PORT;
        self.access_token = FIXED_ACCESS_TOKEN.to_string();
        self.allowed_origins = vec!["*".into()];
        self.sumatra_pdf_path = String::new();
        self.keep_downloaded_files = false;
        self.confirmation_required = false;

        if self.download_dir.trim().is_empty() {
            self.download_dir = config_dir.join("downloads").to_string_lossy().into_owned();
        } else {
            self.download_dir = self.download_dir.trim().to_string();
        }

        self
    }
}

impl BridgeCore {
    pub fn new(settings: BridgeSettings, config_dir: PathBuf) -> Self {
        Self {
            settings: Mutex::new(settings),
            jobs: Mutex::new(VecDeque::new()),
            service_running: Mutex::new(false),
            last_error: Mutex::new(None),
            config_dir,
        }
    }

    pub fn settings(&self) -> BridgeSettings {
        self.settings
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| BridgeSettings::default_for_dir(&self.config_dir))
    }

    pub fn replace_settings(&self, next: BridgeSettings) {
        if let Ok(mut guard) = self.settings.lock() {
            *guard = next;
        }
    }

    pub fn jobs(&self) -> Vec<PrintJob> {
        self.jobs
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn insert_job(&self, job: PrintJob) {
        if let Ok(mut guard) = self.jobs.lock() {
            guard.push_front(job);
            while guard.len() > MAX_RECENT_JOBS {
                guard.pop_back();
            }
        }
    }

    pub fn find_job(&self, job_id: &str) -> Option<PrintJob> {
        self.jobs
            .lock()
            .ok()
            .and_then(|guard| guard.iter().find(|job| job.id == job_id).cloned())
    }

    pub fn update_job<F>(&self, job_id: &str, updater: F) -> Result<PrintJob, String>
    where
        F: FnOnce(&mut PrintJob),
    {
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| "读取打印任务状态失败".to_string())?;
        let job = guard
            .iter_mut()
            .find(|job| job.id == job_id)
            .ok_or_else(|| format!("未找到打印任务: {job_id}"))?;
        updater(job);
        job.updated_at = now_iso();
        Ok(job.clone())
    }

    pub fn set_service_status(&self, running: bool, last_error: Option<String>) {
        if let Ok(mut guard) = self.service_running.lock() {
            *guard = running;
        }
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = last_error;
        }
    }

    pub fn service_status(&self) -> ServiceStatus {
        let settings = self.settings();
        let running = self
            .service_running
            .lock()
            .map(|guard| *guard)
            .unwrap_or(false);
        let last_error = self.last_error.lock().ok().and_then(|guard| guard.clone());

        ServiceStatus {
            running,
            port: settings.port,
            base_url: format!("http://127.0.0.1:{}", settings.port),
            last_error,
        }
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            settings: self.settings(),
            service: self.service_status(),
            jobs: self.jobs(),
            config_dir: self.config_dir.to_string_lossy().into_owned(),
        }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    pub fn settings_path(&self) -> PathBuf {
        self.config_dir.join(SETTINGS_FILE_NAME)
    }

    pub fn apply_settings_input(&self, input: SaveSettingsInput) -> Result<BridgeSettings, String> {
        if !(1024..=65535).contains(&input.port) {
            return Err("端口必须在 1024 到 65535 之间".into());
        }

        let mut next = self.settings();
        next.port = input.port;
        next.allowed_origins = input.allowed_origins;
        next.sumatra_pdf_path = input.sumatra_pdf_path;
        next.download_dir = input.download_dir;
        next.keep_downloaded_files = input.keep_downloaded_files;
        next.confirmation_required = input.confirmation_required;
        let next = next.normalize(&self.config_dir);

        fs::create_dir_all(&next.download_dir)
            .map_err(|error| format!("创建下载目录失败: {error}"))?;
        self.replace_settings(next.clone());
        Ok(next)
    }

    pub fn rotate_access_token(&self) -> BridgeSettings {
        let mut next = self.settings();
        next.access_token = generate_access_token();
        self.replace_settings(next.clone());
        next
    }
}

impl AppState {
    pub fn new(core: Arc<BridgeCore>) -> Self {
        Self { core }
    }
}

pub fn load_state(app: &AppHandle) -> Result<Arc<BridgeCore>, String> {
    let preferred_config_dir = resolve_config_dir(app);
    let config_dir = match fs::create_dir_all(&preferred_config_dir) {
        Ok(()) => preferred_config_dir,
        Err(error) => {
            crate::diagnostics::bootstrap(format!(
                "create config dir failed path={} error={error}",
                preferred_config_dir.display()
            ));
            let fallback = std::env::temp_dir().join("liaoyitong-print-bridge");
            fs::create_dir_all(&fallback).map_err(|fallback_error| {
                format!("创建配置目录失败: {error}; 创建备用配置目录失败: {fallback_error}")
            })?;
            fallback
        }
    };

    let mut settings = BridgeSettings::default_for_dir(&config_dir).normalize(&config_dir);
    if let Err(error) = fs::create_dir_all(&settings.download_dir) {
        crate::diagnostics::bootstrap(format!(
            "create download dir failed path={} error={error}",
            settings.download_dir
        ));
        let fallback_download_dir = std::env::temp_dir()
            .join("liaoyitong-print-bridge")
            .join("downloads");
        fs::create_dir_all(&fallback_download_dir).map_err(|fallback_error| {
            format!("创建下载目录失败: {error}; 创建备用下载目录失败: {fallback_error}")
        })?;
        settings.download_dir = fallback_download_dir.to_string_lossy().into_owned();
    }

    let core = Arc::new(BridgeCore::new(settings, config_dir));
    if let Err(error) = persist_settings(core.as_ref()) {
        crate::diagnostics::bootstrap(format!("persist settings failed: {error}"));
    }
    Ok(core)
}

pub fn persist_settings(core: &BridgeCore) -> Result<(), String> {
    let payload = PersistedSettings {
        version: SETTINGS_VERSION,
        settings: core.settings(),
    };
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("序列化配置失败: {error}"))?;
    fs::write(core.settings_path(), content).map_err(|error| format!("写入配置文件失败: {error}"))
}

pub fn emit_snapshot(app: &AppHandle, core: &Arc<BridgeCore>) {
    let _ = app.emit_all(SNAPSHOT_EVENT, core.snapshot());
}

pub fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn generate_job_id() -> String {
    format!("job_{}", random_suffix(12))
}

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn resolve_config_dir(app: &AppHandle) -> PathBuf {
    path::app_config_dir(&app.config())
        .unwrap_or_else(|| std::env::temp_dir().join("lyt_print_bridge"))
}

fn generate_access_token() -> String {
    FIXED_ACCESS_TOKEN.to_string()
}

fn random_suffix(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}
