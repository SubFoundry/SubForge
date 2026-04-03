use std::collections::BTreeMap;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::helpers::{
    abort_events_task, parse_gui_close_behavior, read_bootstrap_line, resolve_workspace_root,
    spawn_log_reader, terminate_child,
};
use super::types::{CoreState, CoreStatusPayload, GuiCloseBehavior};

pub(crate) struct CoreManager {
    pub(super) workspace_root: std::path::PathBuf,
    pub(super) core_data_dir: std::path::PathBuf,
    pub(super) state: Mutex<CoreState>,
    pub(super) client: reqwest::Client,
}

impl CoreManager {
    pub(crate) fn new() -> Result<Self> {
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

    pub(crate) async fn start_core(&self) -> Result<CoreStatusPayload> {
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
            .arg(super::types::DEFAULT_CORE_HOST)
            .arg("--port")
            .arg(super::types::DEFAULT_CORE_PORT.to_string())
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

    pub(crate) async fn stop_core(&self) -> Result<CoreStatusPayload> {
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

    pub(crate) async fn compose_status_payload(&self) -> Result<CoreStatusPayload> {
        let (base_url, pid, fallback_version) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            self.try_restore_admin_token(&mut state);
            (state.base_url.clone(), state.pid, state.version.clone())
        };

        let healthy_version = self.fetch_health_version(&base_url).await;
        let running = healthy_version.is_some();
        if running {
            let mut state = self.lock_state()?;
            self.try_restore_admin_token(&mut state);
        }

        Ok(CoreStatusPayload {
            running,
            base_url,
            version: healthy_version.or(fallback_version),
            pid,
        })
    }

    pub(super) async fn resolve_gui_close_behavior(&self) -> GuiCloseBehavior {
        let settings = match self.fetch_system_settings().await {
            Ok(settings) => settings,
            Err(_) => return GuiCloseBehavior::TrayMinimize,
        };
        parse_gui_close_behavior(&settings)
    }

    async fn wait_until_healthy(&self, timeout: Duration) -> Result<()> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let base_url = {
                let state = self.lock_state()?;
                state.base_url.clone()
            };

            if self.fetch_health_version(&base_url).await.is_some() {
                return Ok(());
            }

            if std::time::Instant::now() >= deadline {
                return Err(anyhow!("Core 启动超时，/health 未就绪"));
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub(super) async fn fetch_health_version(&self, base_url: &str) -> Option<String> {
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

    pub(super) fn lock_state(&self) -> Result<MutexGuard<'_, CoreState>> {
        self.state
            .lock()
            .map_err(|_| anyhow!("CoreManager 状态锁异常"))
    }

    pub(super) fn reap_child_if_exited(&self, state: &mut CoreState) -> Result<()> {
        if let Some(child) = state.child.as_mut()
            && child
                .try_wait()
                .context("读取 Core 进程状态失败")?
                .is_some()
        {
            state.child = None;
            state.admin_token = None;
            state.pid = None;
            abort_events_task(state);
        }
        Ok(())
    }

    pub(super) fn try_restore_admin_token(&self, state: &mut CoreState) {
        if state.admin_token.is_some() {
            return;
        }

        let token_path = self.core_data_dir.join("admin_token");
        let token = fs::read_to_string(&token_path)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if let Some(token) = token {
            state.admin_token = Some(token);
        }
    }

    pub(super) async fn fetch_system_settings(&self) -> Result<BTreeMap<String, String>> {
        let response = self
            .proxy_api_call(super::types::CoreApiRequest {
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

        let payload: super::types::SettingsResponse =
            serde_json::from_str(&response.body).context("解析系统设置响应失败")?;
        Ok(payload.settings)
    }
}
