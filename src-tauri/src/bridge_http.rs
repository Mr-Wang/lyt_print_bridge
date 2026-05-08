use serde::Serialize;
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::AppHandle;

use crate::{
    app_state::{emit_snapshot, AppState, PrintRequestPayload, FIXED_ACCESS_TOKEN},
    print_service, ServiceState,
};

const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;

pub struct LocalHttpHandle {
    port: u16,
    running: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl LocalHttpHandle {
    pub fn start(app: AppHandle, state: AppState, port: u16) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|error| format!("failed to bind 127.0.0.1:{port}: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("failed to set listener nonblocking: {error}"))?;

        let running = Arc::new(AtomicBool::new(true));
        let worker_running = running.clone();
        let join_handle = thread::spawn(move || {
            while worker_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let request_app = app.clone();
                        let request_state = state.clone();
                        thread::spawn(move || {
                            if let Err(error) =
                                handle_connection(stream, &request_app, &request_state)
                            {
                                request_state.core.set_service_status(
                                    true,
                                    Some(format!("request handler error: {error}")),
                                );
                                emit_snapshot(&request_app, &request_state.core);
                            }
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(120));
                    }
                    Err(error) => {
                        state
                            .core
                            .set_service_status(true, Some(format!("accept error: {error}")));
                        emit_snapshot(&app, &state.core);
                        thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        });

        Ok(Self {
            port,
            running,
            join_handle: Some(join_handle),
        })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for LocalHttpHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn restart_service(
    app: &AppHandle,
    state: &AppState,
    service: &ServiceState,
) -> Result<(), String> {
    if let Ok(mut guard) = service.handle.lock() {
        if let Some(mut handle) = guard.take() {
            handle.stop();
        }
    }

    let port = state.core.settings().port;
    match LocalHttpHandle::start(app.clone(), state.clone(), port) {
        Ok(handle) => {
            if let Ok(mut guard) = service.handle.lock() {
                *guard = Some(handle);
            }
            state.core.set_service_status(true, None);
            emit_snapshot(app, &state.core);
            Ok(())
        }
        Err(error) => {
            state.core.set_service_status(false, Some(error.clone()));
            emit_snapshot(app, &state.core);
            Err(error)
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("failed to set read timeout: {error}"))?;

    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            crate::diagnostics::write(app, "bridge_http", format!("read_request failed: {error}"));
            let response = error_response(400, &error, Some("*".into()));
            return write_response(&mut stream, response);
        }
    };
    let origin = request.header("origin").map(|value| value.to_string());
    let allowed = origin
        .as_deref()
        .map(|value| is_origin_allowed(&state.core.settings().allowed_origins, value))
        .unwrap_or(true);
    let cors_origin = if allowed {
        Some(origin.clone().unwrap_or_else(|| "*".into()))
    } else {
        None
    };

    crate::diagnostics::write(
        app,
        "bridge_http",
        format!(
            "request method={} path={} origin={} allowed={} body_bytes={}",
            request.method,
            request.path,
            origin.as_deref().unwrap_or("<none>"),
            allowed,
            request.body.len()
        ),
    );

    let response = if !allowed {
        error_response(
            403,
            "origin not allowed. 请把当前页面 Origin 加入 allowed origins，或使用新版默认配置。",
            origin,
        )
    } else {
        dispatch_request(app, state, request, cors_origin.clone())
    };

    write_response(&mut stream, response)
}

fn dispatch_request(
    app: &AppHandle,
    state: &AppState,
    request: HttpRequest,
    cors_origin: Option<String>,
) -> HttpResponse {
    if request.method == "OPTIONS" {
        return empty_response(204, cors_origin);
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") | ("GET", "/status") => status_page_response(cors_origin),
        ("GET", "/printer/ping") => json_response(
            200,
            &json!({
                "ok": true,
                "service": state.core.service_status(),
                "version": env!("CARGO_PKG_VERSION"),
            }),
            cors_origin,
        ),
        ("GET", "/printer/list") => {
            if !is_authorized(&state.core.settings().access_token, &request) {
                return error_response(401, "missing or invalid bridge token", cors_origin);
            }
            match print_service::list_printers() {
                Ok(printers) => json_response(200, &json!({ "printers": printers }), cors_origin),
                Err(error) => error_response(500, &error, cors_origin),
            }
        }
        ("GET", "/printer/jobs") => {
            if !is_authorized(&state.core.settings().access_token, &request) {
                return error_response(401, "missing or invalid bridge token", cors_origin);
            }
            json_response(200, &json!({ "jobs": state.core.jobs() }), cors_origin)
        }
        ("POST", "/printer/print") => {
            if !is_authorized(&state.core.settings().access_token, &request) {
                return error_response(401, "missing or invalid bridge token", cors_origin);
            }
            match serde_json::from_slice::<PrintRequestPayload>(&request.body) {
                Ok(payload) => match print_service::submit_http_print(app, &state.core, payload) {
                    Ok(job) => json_response(202, &job, cors_origin),
                    Err(error) => error_response(400, &error, cors_origin),
                },
                Err(error) => {
                    error_response(400, &format!("invalid JSON body: {error}"), cors_origin)
                }
            }
        }
        _ if request.method == "GET"
            && request.path.starts_with("/printer/jobs/")
            && request.path.ends_with("/document") =>
        {
            if !is_authorized(&state.core.settings().access_token, &request) {
                return error_response(401, "missing or invalid bridge token", cors_origin);
            }

            let job_id = request
                .path
                .trim_start_matches("/printer/jobs/")
                .trim_end_matches("/document")
                .trim_end_matches('/');
            match state.core.find_job(job_id) {
                Some(job) => match job.local_file_path {
                    Some(path) => match fs::read(&path) {
                        Ok(bytes) => binary_response(
                            200,
                            "application/pdf".into(),
                            bytes,
                            cors_origin,
                        ),
                        Err(error) => error_response(
                            404,
                            &format!("document file not readable: {error}"),
                            cors_origin,
                        ),
                    },
                    None => error_response(404, "job document not ready", cors_origin),
                },
                None => error_response(404, "job not found", cors_origin),
            }
        }
        _ if request.method == "GET" && request.path.starts_with("/printer/jobs/") => {
            if !is_authorized(&state.core.settings().access_token, &request) {
                return error_response(401, "missing or invalid bridge token", cors_origin);
            }
            let job_id = request.path.trim_start_matches("/printer/jobs/");
            match state.core.find_job(job_id) {
                Some(job) => json_response(200, &job, cors_origin),
                None => error_response(404, "job not found", cors_origin),
            }
        }
        _ => error_response(404, "not found", cors_origin),
    }
}

fn is_origin_allowed(allowed_origins: &[String], origin: &str) -> bool {
    allowed_origins
        .iter()
        .any(|value| value == "*" || value == origin)
}

fn is_authorized(expected_token: &str, request: &HttpRequest) -> bool {
    request
        .header("x-print-bridge-token")
        .map(|value| value == expected_token)
        .or_else(|| {
            request.header("authorization").and_then(|value| {
                value
                    .strip_prefix("Bearer ")
                    .map(|token| token == expected_token)
            })
        })
        .or_else(|| request.query_param("token").map(|value| value == expected_token))
        .unwrap_or(false)
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::with_capacity(4096);
    let mut temp = [0u8; 4096];
    let mut body_start = None;
    let mut content_length = 0usize;

    loop {
        let size = stream
            .read(&mut temp)
            .map_err(|error| format!("failed to read request: {error}"))?;
        if size == 0 {
            break;
        }

        buffer.extend_from_slice(&temp[..size]);
        if body_start.is_none() {
            if let Some(start) = find_body_start(&buffer) {
                content_length = parse_content_length(&buffer[..start])?;
                if content_length > MAX_REQUEST_BYTES {
                    return Err(format!(
                        "request too large: max {} MB. 如果是本地 PDF，请选择更小文件或改用网络 PDF 地址。",
                        MAX_REQUEST_BYTES / 1024 / 1024
                    ));
                }
                body_start = Some(start);
                if buffer.len() >= start + content_length {
                    break;
                }
            }
        } else if let Some(start) = body_start {
            if buffer.len() >= start + content_length {
                break;
            }
        }

        if content_length > MAX_REQUEST_BYTES || buffer.len() > MAX_REQUEST_BYTES {
            return Err(format!(
                "request too large: max {} MB. 如果是本地 PDF，请选择更小文件或改用网络 PDF 地址。",
                MAX_REQUEST_BYTES / 1024 / 1024
            ));
        }
    }

    let body_start = body_start.ok_or_else(|| "malformed HTTP request".to_string())?;
    if buffer.len() < body_start + content_length {
        return Err("incomplete request body".into());
    }

    let header_text = String::from_utf8_lossy(&buffer[..body_start]);
    let mut lines = header_text.lines();
    let first_line = lines
        .next()
        .ok_or_else(|| "empty HTTP request".to_string())?;
    let mut parts = first_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_string())?;
    let target = parts
        .next()
        .ok_or_else(|| "missing HTTP target".to_string())?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.to_string();
    let query = query.to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    Ok(HttpRequest {
        method: method.to_string(),
        path,
        query,
        headers,
        body: buffer[body_start..body_start + content_length].to_vec(),
    })
}

fn find_body_start(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let header_text = String::from_utf8_lossy(header_bytes);
    for line in header_text.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid Content-Length header: {error}"));
            }
        }
    }
    Ok(0)
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), String> {
    let status_text = match response.status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };

    let mut header_text = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        status_text,
        response.content_type,
        response.body.len()
    );

    if let Some(origin) = response.cors_origin {
        header_text.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\n"));
        header_text.push_str("Vary: Origin\r\n");
        header_text.push_str(
            "Access-Control-Allow-Headers: Content-Type, Authorization, X-Print-Bridge-Token\r\n",
        );
        header_text.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
    }

    header_text.push_str("\r\n");

    stream
        .write_all(header_text.as_bytes())
        .map_err(|error| format!("failed to write response headers: {error}"))?;
    if !response.body.is_empty() {
        stream
            .write_all(&response.body)
            .map_err(|error| format!("failed to write response body: {error}"))?;
    }
    Ok(())
}

fn json_response<T: Serialize>(
    status: u16,
    payload: &T,
    cors_origin: Option<String>,
) -> HttpResponse {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    HttpResponse {
        status,
        content_type: "application/json; charset=utf-8".into(),
        body,
        cors_origin,
    }
}

fn binary_response(
    status: u16,
    content_type: String,
    body: Vec<u8>,
    cors_origin: Option<String>,
) -> HttpResponse {
    HttpResponse {
        status,
        content_type,
        body,
        cors_origin,
    }
}

fn html_response(status: u16, body: String, cors_origin: Option<String>) -> HttpResponse {
    HttpResponse {
        status,
        content_type: "text/html; charset=utf-8".into(),
        body: body.into_bytes(),
        cors_origin,
    }
}

fn status_page_response(cors_origin: Option<String>) -> HttpResponse {
    html_response(200, status_page_html(), cors_origin)
}

fn error_response(status: u16, message: &str, cors_origin: Option<String>) -> HttpResponse {
    json_response(status, &json!({ "error": message }), cors_origin)
}

fn empty_response(status: u16, cors_origin: Option<String>) -> HttpResponse {
    HttpResponse {
        status,
        content_type: "text/plain; charset=utf-8".into(),
        body: Vec::new(),
        cors_origin,
    }
}

struct HttpRequest {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(|value| value.as_str())
    }

    fn query_param(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            if key == name {
                Some(value)
            } else {
                None
            }
        })
    }
}

struct HttpResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    cors_origin: Option<String>,
}

fn status_page_html() -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
      <title>辽易通打印桥 v{version}</title>
  <style>
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      background: #f6f8fb;
      color: #172033;
      font-family: "Microsoft YaHei", "Segoe UI", sans-serif;
    }}
    main {{
      width: min(760px, calc(100vw - 32px));
      margin: 28px auto;
    }}
    h1 {{
      margin: 0 0 6px;
      font-size: 22px;
      font-weight: 700;
    }}
    .muted {{ color: #637083; }}
    .grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 12px;
      margin: 18px 0;
    }}
    .card {{
      background: #fff;
      border: 1px solid #d9e1ec;
      border-radius: 8px;
      padding: 14px;
    }}
    .card span {{
      display: block;
      color: #637083;
      font-size: 12px;
      margin-bottom: 6px;
    }}
    .card strong {{
      font-size: 18px;
    }}
    button {{
      border: 1px solid #bac7d6;
      border-radius: 6px;
      background: #fff;
      color: #172033;
      padding: 8px 12px;
      cursor: pointer;
      margin-right: 8px;
    }}
    pre {{
      min-height: 220px;
      overflow: auto;
      background: #101827;
      color: #d7e1ef;
      border-radius: 8px;
      padding: 12px;
      line-height: 1.45;
      white-space: pre-wrap;
    }}
    .ok {{ color: #087f5b; }}
    .bad {{ color: #c92a2a; }}
  </style>
</head>
<body>
  <main>
    <h1>辽易通打印桥 v{version}</h1>
    <section class="grid">
      <div class="card"><span>服务状态</span><strong id="service">检测中</strong></div>
      <div class="card"><span>本地地址</span><strong id="base-url">-</strong></div>
      <div class="card"><span>最近任务</span><strong id="job-count">-</strong></div>
    </section>
    <p>
      <button id="refresh">刷新</button>
      <button id="copy">复制诊断信息</button>
    </p>
    <pre id="log">正在读取本地服务...</pre>
  </main>
  <script>
    var token = "{token}";
    var logEl = document.getElementById('log');
    function write(line) {{
      var time = new Date().toISOString();
      logEl.textContent += "\n[" + time + "] " + line;
      logEl.scrollTop = logEl.scrollHeight;
      console.log('[browser-status]', line);
    }}
    async function getJson(path, options) {{
      var response = await fetch(path, options || {{}});
      var text = await response.text();
      try {{
        return {{ ok: response.ok, status: response.status, body: JSON.parse(text) }};
      }} catch (error) {{
        return {{ ok: response.ok, status: response.status, body: text }};
      }}
    }}
    async function refresh() {{
      logEl.textContent = '';
      write('refresh started: ' + location.href);
      try {{
        var ping = await getJson('/printer/ping');
        write('GET /printer/ping HTTP ' + ping.status + ' ' + JSON.stringify(ping.body));
        var service = ping.body && ping.body.service ? ping.body.service : null;
        document.getElementById('service').textContent = service && service.running ? '运行中' : '异常';
        document.getElementById('service').className = service && service.running ? 'ok' : 'bad';
        document.getElementById('base-url').textContent = service && service.baseUrl ? service.baseUrl : location.origin;

        var jobs = await getJson('/printer/jobs', {{
          headers: {{ 'X-Print-Bridge-Token': token }}
        }});
        write('GET /printer/jobs HTTP ' + jobs.status + ' ' + JSON.stringify(jobs.body));
        document.getElementById('job-count').textContent = jobs.body && jobs.body.jobs ? jobs.body.jobs.length : '0';
      }} catch (error) {{
        write('诊断页请求失败: ' + (error && error.stack ? error.stack : error));
        document.getElementById('service').textContent = '异常';
        document.getElementById('service').className = 'bad';
      }}
    }}
    document.getElementById('refresh').onclick = refresh;
    document.getElementById('copy').onclick = async function () {{
      await navigator.clipboard.writeText(logEl.textContent);
      write('诊断信息已复制');
    }};
    refresh();
  </script>
</body>
</html>"#,
        version = env!("CARGO_PKG_VERSION"),
        token = FIXED_ACCESS_TOKEN
    )
}
