use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

const DEFAULT_CORE_HOST: &str = "127.0.0.1";
const DEFAULT_CORE_PORT: u16 = 18118;

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreApiResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CoreBootstrapLine {
    version: String,
    listen_addr: String,
    port: u16,
    admin_token: String,
}

#[derive(Debug)]
struct CoreState {
    child: Option<Child>,
    admin_token: Option<String>,
    base_url: String,
    version: Option<String>,
    pid: Option<u32>,
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            child: None,
            admin_token: None,
            base_url: default_base_url(),
            version: None,
            pid: None,
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

fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
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

fn main() {
    let manager = CoreManager::new().expect("初始化 CoreManager 失败");

    tauri::Builder::default()
        .manage(manager)
        .invoke_handler(tauri::generate_handler![
            core_start,
            core_stop,
            core_status,
            core_api_call
        ])
        .run(tauri::generate_context!())
        .expect("运行 SubForge Desktop 失败");
}
