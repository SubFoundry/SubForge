use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::task::JoinHandle;

const DEFAULT_CORE_HOST: &str = "127.0.0.1";
const DEFAULT_CORE_PORT: u16 = 18118;
const MAX_PLUGIN_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_GUI_CLOSE_BEHAVIOR: GuiCloseBehavior = GuiCloseBehavior::TrayMinimize;
const SETTING_KEY_GUI_CLOSE_BEHAVIOR: &str = "gui_close_behavior";
const SETTING_KEY_TRAY_MINIMIZE: &str = "tray_minimize";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiCloseBehavior {
    TrayMinimize,
    CloseGui,
    CloseGuiAndStopCore,
}

impl GuiCloseBehavior {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tray_minimize" => Some(Self::TrayMinimize),
            "close_gui" => Some(Self::CloseGui),
            "close_gui_and_stop_core" => Some(Self::CloseGuiAndStopCore),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreStatusPayload {
    running: bool,
    base_url: String,
    version: Option<String>,
    pid: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreApiRequest {
    method: String,
    path: String,
    body: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginImportRequest {
    file_name: String,
    payload_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreApiResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct SettingsResponse {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct CoreBootstrapLine {
    version: String,
    listen_addr: String,
    port: u16,
    admin_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreEventPayload {
    event: String,
    message: String,
    source_id: Option<String>,
    timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreBridgeEvent {
    kind: String,
    payload: Option<CoreEventPayload>,
    message: Option<String>,
}

#[derive(Debug)]
struct CoreState {
    child: Option<Child>,
    admin_token: Option<String>,
    base_url: String,
    version: Option<String>,
    pid: Option<u32>,
    events_task: Option<JoinHandle<()>>,
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            child: None,
            admin_token: None,
            base_url: default_base_url(),
            version: None,
            pid: None,
            events_task: None,
        }
    }
}

struct CoreManager {
    workspace_root: PathBuf,
    core_data_dir: PathBuf,
    state: Mutex<CoreState>,
    client: reqwest::Client,
}

impl CoreManager {
    fn new() -> Result<Self> {
        let workspace_root = resolve_workspace_root()?;
        let core_data_dir = workspace_root.join(".subforge-desktop");
        fs::create_dir_all(&core_data_dir).with_context(|| {
            format!(
                "创建 Desktop Core 数据目录失败: {}",
                core_data_dir.display()
            )
        })?;

        Ok(Self {
            workspace_root,
            core_data_dir,
            state: Mutex::new(CoreState::default()),
            client: reqwest::Client::new(),
        })
    }

    async fn start_core(&self) -> Result<CoreStatusPayload> {
        let already_running = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            state.child.is_some()
        };

        if already_running {
            return self.compose_status_payload().await;
        }

        let mut command = Command::new("cargo");
        command
            .current_dir(&self.workspace_root)
            .arg("run")
            .arg("-p")
            .arg("subforge-core")
            .arg("--")
            .arg("run")
            .arg("--host")
            .arg(DEFAULT_CORE_HOST)
            .arg("--port")
            .arg(DEFAULT_CORE_PORT.to_string())
            .arg("--gui-mode")
            .arg("--data-dir")
            .arg(&self.core_data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().context("启动 subforge-core 失败")?;
        let stdout = child
            .stdout
            .take()
            .context("读取 subforge-core stdout 失败")?;
        let stderr = child
            .stderr
            .take()
            .context("读取 subforge-core stderr 失败")?;

        let bootstrap = read_bootstrap_line(stdout)?;
        spawn_log_reader(stderr, "core-stderr");

        {
            let mut state = self.lock_state()?;
            state.base_url = format!("http://{}:{}", bootstrap.listen_addr, bootstrap.port);
            state.version = Some(bootstrap.version);
            state.pid = Some(child.id());
            state.admin_token = Some(bootstrap.admin_token);
            state.child = Some(child);
        }

        self.wait_until_healthy(Duration::from_secs(5)).await?;
        self.compose_status_payload().await
    }

    async fn stop_core(&self) -> Result<CoreStatusPayload> {
        let mut maybe_child = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            abort_events_task(&mut state);
            state.child.take()
        };

        if let Some(child) = maybe_child.as_mut() {
            terminate_child(child).context("停止 Core 进程失败")?;
        }

        {
            let mut state = self.lock_state()?;
            state.admin_token = None;
            state.pid = None;
            state.version = None;
        }

        self.compose_status_payload().await
    }

    fn start_events_bridge(&self, app_handle: AppHandle) -> Result<()> {
        let (base_url, admin_token, core_running) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            let can_start_new_bridge = if let Some(task) = state.events_task.as_ref() {
                task.is_finished()
            } else {
                true
            };
            if !can_start_new_bridge {
                return Ok(());
            }
            abort_events_task(&mut state);
            (
                state.base_url.clone(),
                state.admin_token.clone(),
                state.child.is_some(),
            )
        };

        if !core_running {
            return Ok(());
        }

        if admin_token.is_none() {
            emit_bridge_event(
                &app_handle,
                CoreBridgeEvent {
                    kind: "disconnected".to_string(),
                    payload: None,
                    message: Some("Core 事件流未启动（缺少 token 或 Core 未运行）".to_string()),
                },
            );
            return Ok(());
        }

        let token = admin_token.expect("admin_token 已判空");
        let url = format!("{base_url}/api/events");
        let client = self.client.clone();
        let task = tokio::spawn(async move {
            emit_bridge_event(
                &app_handle,
                CoreBridgeEvent {
                    kind: "connected".to_string(),
                    payload: None,
                    message: Some("Core 事件流已连接".to_string()),
                },
            );

            let response = match client.get(&url).bearer_auth(token).send().await {
                Ok(response) => response,
                Err(error) => {
                    emit_bridge_event(
                        &app_handle,
                        CoreBridgeEvent {
                            kind: "error".to_string(),
                            payload: None,
                            message: Some(format!("Core 事件流连接失败：{error}")),
                        },
                    );
                    emit_bridge_event(
                        &app_handle,
                        CoreBridgeEvent {
                            kind: "disconnected".to_string(),
                            payload: None,
                            message: Some("Core 事件流已断开".to_string()),
                        },
                    );
                    return;
                }
            };

            if !response.status().is_success() {
                emit_bridge_event(
                    &app_handle,
                    CoreBridgeEvent {
                        kind: "error".to_string(),
                        payload: None,
                        message: Some(format!(
                            "Core 事件流连接失败，HTTP 状态码：{}",
                            response.status()
                        )),
                    },
                );
                emit_bridge_event(
                    &app_handle,
                    CoreBridgeEvent {
                        kind: "disconnected".to_string(),
                        payload: None,
                        message: Some("Core 事件流已断开".to_string()),
                    },
                );
                return;
            }

            let mut event_name = String::new();
            let mut data_lines: Vec<String> = Vec::new();
            let mut buffer = String::new();
            let mut response = response;

            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(error) => {
                        emit_bridge_event(
                            &app_handle,
                            CoreBridgeEvent {
                                kind: "error".to_string(),
                                payload: None,
                                message: Some(format!("Core 事件流读取失败：{error}")),
                            },
                        );
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(idx) = buffer.find('\n') {
                    let mut line = buffer[..idx].to_string();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    buffer = buffer[idx + 1..].to_string();

                    if line.is_empty() {
                        if !data_lines.is_empty() {
                            let data = data_lines.join("\n");
                            if !data.eq_ignore_ascii_case("keepalive") {
                                let emitted_event = parse_core_event_payload(&event_name, &data);
                                emit_bridge_event(
                                    &app_handle,
                                    CoreBridgeEvent {
                                        kind: "event".to_string(),
                                        payload: Some(emitted_event),
                                        message: None,
                                    },
                                );
                            }
                            event_name.clear();
                            data_lines.clear();
                        }
                        continue;
                    }

                    if line.starts_with(':') {
                        continue;
                    }

                    if let Some(raw) = line.strip_prefix("event:") {
                        event_name = raw.trim().to_string();
                        continue;
                    }

                    if let Some(raw) = line.strip_prefix("data:") {
                        data_lines.push(raw.trim_start().to_string());
                    }
                }
            }

            emit_bridge_event(
                &app_handle,
                CoreBridgeEvent {
                    kind: "disconnected".to_string(),
                    payload: None,
                    message: Some("Core 事件流已断开".to_string()),
                },
            );
        });

        let mut state = self.lock_state()?;
        state.events_task = Some(task);
        Ok(())
    }

    async fn compose_status_payload(&self) -> Result<CoreStatusPayload> {
        let (base_url, pid, fallback_version) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            (state.base_url.clone(), state.pid, state.version.clone())
        };

        let healthy_version = self.fetch_health_version(&base_url).await;
        let running = healthy_version.is_some();

        Ok(CoreStatusPayload {
            running,
            base_url,
            version: healthy_version.or(fallback_version),
            pid,
        })
    }

    async fn proxy_api_call(&self, request: CoreApiRequest) -> Result<CoreApiResponse> {
        let (base_url, admin_token) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            (state.base_url.clone(), state.admin_token.clone())
        };

        if self.fetch_health_version(&base_url).await.is_none() {
            return Err(anyhow!("Core 未运行或不可达"));
        }

        let path = normalize_path(&request.path);
        if path.starts_with("/api/") && admin_token.is_none() {
            return Err(anyhow!(
                "当前会话没有管理 token，请先通过 GUI 启动 Core 再调用管理 API"
            ));
        }

        let method = Method::from_bytes(request.method.as_bytes())
            .with_context(|| format!("不支持的 HTTP 方法: {}", request.method))?;
        let url = format!("{base_url}{path}");

        let mut builder = self.client.request(method, &url);
        if let Some(token) = admin_token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }

        let response = builder
            .send()
            .await
            .with_context(|| format!("调用 Core API 失败: {url}"))?;
        let status = response.status().as_u16();

        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (key.to_string(), v.to_string()))
            })
            .collect::<BTreeMap<_, _>>();

        let body = response.text().await.context("读取 Core API 响应失败")?;

        Ok(CoreApiResponse {
            status,
            headers,
            body,
        })
    }

    async fn import_plugin_zip(&self, request: PluginImportRequest) -> Result<CoreApiResponse> {
        if !request.file_name.to_ascii_lowercase().ends_with(".zip") {
            return Err(anyhow!("仅支持 .zip 插件包"));
        }

        let payload = BASE64_STANDARD
            .decode(request.payload_base64.as_bytes())
            .context("解析插件包内容失败（Base64）")?;
        if payload.len() > MAX_PLUGIN_UPLOAD_BYTES {
            return Err(anyhow!(
                "插件包超过大小限制：{} bytes",
                MAX_PLUGIN_UPLOAD_BYTES
            ));
        }

        let (base_url, admin_token) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            (state.base_url.clone(), state.admin_token.clone())
        };

        if self.fetch_health_version(&base_url).await.is_none() {
            return Err(anyhow!("Core 未运行或不可达"));
        }

        let token = admin_token.ok_or_else(|| {
            anyhow!("当前会话没有管理 token，请先通过 GUI 启动 Core 再调用管理 API")
        })?;

        let boundary_seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let boundary = format!("----subforge-desktop-{boundary_seed}");
        let multipart_body = build_plugin_multipart_body(&boundary, &payload, &request.file_name);
        let response = self
            .client
            .request(Method::POST, format!("{base_url}/api/plugins/import"))
            .bearer_auth(token)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(multipart_body)
            .send()
            .await
            .context("调用 Core 插件导入接口失败")?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (key.to_string(), v.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let body = response.text().await.context("读取插件导入响应失败")?;

        Ok(CoreApiResponse {
            status,
            headers,
            body,
        })
    }

    async fn resolve_gui_close_behavior(&self) -> GuiCloseBehavior {
        let settings = match self.fetch_system_settings().await {
            Ok(settings) => settings,
            Err(_) => return DEFAULT_GUI_CLOSE_BEHAVIOR,
        };
        parse_gui_close_behavior(&settings)
    }

    async fn fetch_system_settings(&self) -> Result<BTreeMap<String, String>> {
        let response = self
            .proxy_api_call(CoreApiRequest {
                method: "GET".to_string(),
                path: "/api/system/settings".to_string(),
                body: None,
            })
            .await?;

        if response.status != 200 {
            return Err(anyhow!(
                "读取 /api/system/settings 失败，HTTP 状态码: {}",
                response.status
            ));
        }

        let payload: SettingsResponse =
            serde_json::from_str(&response.body).context("解析系统设置响应失败")?;
        Ok(payload.settings)
    }

    async fn wait_until_healthy(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let base_url = {
                let state = self.lock_state()?;
                state.base_url.clone()
            };

            if self.fetch_health_version(&base_url).await.is_some() {
                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(anyhow!("Core 启动超时，/health 未就绪"));
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn fetch_health_version(&self, base_url: &str) -> Option<String> {
        let url = format!("{base_url}/health");
        let response = self.client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }

        let value = response.json::<Value>().await.ok()?;
        value
            .get("version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, CoreState>> {
        self.state
            .lock()
            .map_err(|_| anyhow!("CoreManager 状态锁异常"))
    }

    fn reap_child_if_exited(&self, state: &mut CoreState) -> Result<()> {
        if let Some(child) = state.child.as_mut() {
            if child
                .try_wait()
                .context("读取 Core 进程状态失败")?
                .is_some()
            {
                state.child = None;
                state.admin_token = None;
                state.pid = None;
                abort_events_task(state);
            }
        }
        Ok(())
    }
}

#[tauri::command]
async fn core_start(manager: State<'_, CoreManager>) -> Result<CoreStatusPayload, String> {
    manager.start_core().await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn core_stop(manager: State<'_, CoreManager>) -> Result<CoreStatusPayload, String> {
    manager.stop_core().await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn core_status(manager: State<'_, CoreManager>) -> Result<CoreStatusPayload, String> {
    manager
        .compose_status_payload()
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn core_api_call(
    manager: State<'_, CoreManager>,
    request: CoreApiRequest,
) -> Result<CoreApiResponse, String> {
    manager
        .proxy_api_call(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn core_import_plugin_zip(
    manager: State<'_, CoreManager>,
    request: PluginImportRequest,
) -> Result<CoreApiResponse, String> {
    manager
        .import_plugin_zip(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn core_events_start(
    manager: State<'_, CoreManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    manager
        .start_events_bridge(app_handle)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn desktop_auto_close_gui(app_handle: AppHandle) -> Result<(), String> {
    close_main_window(app_handle);
    Ok(())
}

fn default_base_url() -> String {
    format!("http://{DEFAULT_CORE_HOST}:{DEFAULT_CORE_PORT}")
}

fn resolve_workspace_root() -> Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..");
    root.canonicalize()
        .with_context(|| format!("定位 workspace 根目录失败: {}", root.display()))
}

fn read_bootstrap_line(stdout: std::process::ChildStdout) -> Result<CoreBootstrapLine> {
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    let read_bytes = reader
        .read_line(&mut first_line)
        .context("读取 Core 引导输出失败")?;

    if read_bytes == 0 {
        return Err(anyhow!("Core 启动输出为空"));
    }

    let bootstrap: CoreBootstrapLine =
        serde_json::from_str(first_line.trim()).context("解析 Core 引导 JSON 失败")?;

    spawn_log_reader(reader, "core-stdout");
    Ok(bootstrap)
}

fn spawn_log_reader<R>(reader: R, stream_name: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut line_reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            match line_reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        println!("[{}] {}", stream_name, trimmed);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn parse_core_event_payload(event_name: &str, data: &str) -> CoreEventPayload {
    let fallback_event = if event_name.trim().is_empty() {
        "message".to_string()
    } else {
        event_name.trim().to_string()
    };

    let parsed: Result<Value, _> = serde_json::from_str(data);
    match parsed {
        Ok(value) => {
            let event = value
                .get("event")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or(fallback_event);
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| data.to_string());
            let source_id = value
                .get("source_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let timestamp = value
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            CoreEventPayload {
                event,
                message,
                source_id,
                timestamp,
            }
        }
        Err(_) => CoreEventPayload {
            event: fallback_event,
            message: data.to_string(),
            source_id: None,
            timestamp: None,
        },
    }
}

fn emit_bridge_event(app_handle: &AppHandle, event: CoreBridgeEvent) {
    let _ = app_handle.emit("core://event", event);
}

fn abort_events_task(state: &mut CoreState) {
    if let Some(task) = state.events_task.take() {
        task.abort();
    }
}

fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn build_plugin_multipart_body(boundary: &str, payload: &[u8], file_name: &str) -> Vec<u8> {
    let safe_file_name = file_name
        .replace('\r', "_")
        .replace('\n', "_")
        .replace('"', "_");

    let mut body = Vec::new();
    write!(body, "--{boundary}\r\n").expect("写入 multipart 边界失败");
    write!(
        body,
        "Content-Disposition: form-data; name=\"file\"; filename=\"{safe_file_name}\"\r\n"
    )
    .expect("写入 multipart disposition 失败");
    write!(body, "Content-Type: application/zip\r\n\r\n")
        .expect("写入 multipart content-type 失败");
    body.extend_from_slice(payload);
    write!(body, "\r\n--{boundary}--\r\n").expect("写入 multipart 结束边界失败");
    body
}

fn parse_gui_close_behavior(settings: &BTreeMap<String, String>) -> GuiCloseBehavior {
    if let Some(raw_behavior) = settings.get(SETTING_KEY_GUI_CLOSE_BEHAVIOR) {
        if let Some(parsed) = GuiCloseBehavior::parse(raw_behavior) {
            return parsed;
        }
    }

    if settings
        .get(SETTING_KEY_TRAY_MINIMIZE)
        .is_some_and(|value| parse_bool_setting(value))
    {
        return GuiCloseBehavior::TrayMinimize;
    }

    GuiCloseBehavior::CloseGui
}

fn parse_bool_setting(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("true")
}

fn terminate_child(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;

        let pid = child.id();
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .with_context(|| format!("发送 SIGTERM 失败，pid={pid}"))?;
    }

    #[cfg(windows)]
    {
        child.kill().context("Windows 下终止 Core 进程失败")?;
    }

    let wait_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < wait_deadline {
        if child
            .try_wait()
            .context("等待 Core 进程退出失败")?
            .is_some()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().context("Core 进程未按时退出，强制结束失败")?;
    let _ = child.wait();
    Ok(())
}

async fn apply_main_window_close_behavior(app_handle: AppHandle) {
    let manager = app_handle.state::<CoreManager>();
    let behavior = manager.resolve_gui_close_behavior().await;

    let Some(window) = app_handle.get_webview_window("main") else {
        return;
    };

    match behavior {
        GuiCloseBehavior::TrayMinimize => {
            if window.minimize().is_err() {
                let _ = window.hide();
            }
        }
        GuiCloseBehavior::CloseGui => {
            close_main_window(app_handle);
        }
        GuiCloseBehavior::CloseGuiAndStopCore => {
            let _ = manager.stop_core().await;
            close_main_window(app_handle);
        }
    }
}

fn close_main_window(app_handle: AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.destroy();
    }
}

fn main() {
    let manager = CoreManager::new().expect("初始化 CoreManager 失败");

    tauri::Builder::default()
        .manage(manager)
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    apply_main_window_close_behavior(app_handle).await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            core_start,
            core_stop,
            core_status,
            core_api_call,
            core_import_plugin_zip,
            core_events_start,
            desktop_auto_close_gui
        ])
        .run(tauri::generate_context!())
        .expect("运行 SubForge Desktop 失败");
}
