use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use super::types::{
    CoreBootstrapLine, CoreBridgeEvent, CoreEventPayload, CoreState, GuiCloseBehavior,
    SETTING_KEY_GUI_CLOSE_BEHAVIOR, SETTING_KEY_TRAY_MINIMIZE,
};

pub(super) fn resolve_workspace_root() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..");
    let canonicalized = root.canonicalize().ok()?;
    if !canonicalized.join("Cargo.toml").is_file() {
        return None;
    }
    Some(canonicalized)
}

pub(super) fn resolve_core_data_dir(workspace_root: Option<&Path>) -> Result<PathBuf> {
    if let Ok(from_env) = std::env::var("SUBFORGE_DESKTOP_DATA_DIR") {
        let env_path = PathBuf::from(from_env.trim());
        if !env_path.as_os_str().is_empty() {
            return Ok(env_path);
        }
    }

    if let Some(root) = workspace_root {
        if cfg!(debug_assertions) {
            return Ok(root.join("target").join(".subforge-desktop-dev"));
        }
        return Ok(root.join(".subforge-desktop"));
    }

    #[cfg(windows)]
    {
        let app_data = std::env::var("APPDATA").context("未找到 APPDATA 环境变量")?;
        return Ok(PathBuf::from(app_data).join("SubForge"));
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("未找到 HOME 环境变量")?;
        return Ok(PathBuf::from(home).join("Library/Application Support/SubForge"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME")
            && !xdg_data_home.trim().is_empty()
        {
            return Ok(PathBuf::from(xdg_data_home).join("subforge"));
        }

        let home = std::env::var("HOME").context("未找到 HOME 环境变量")?;
        return Ok(PathBuf::from(home).join(".local/share/subforge"));
    }

    #[allow(unreachable_code)]
    Err(anyhow!("无法确定 Desktop Core 数据目录"))
}

pub(super) fn read_bootstrap_line(stdout: std::process::ChildStdout) -> Result<CoreBootstrapLine> {
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

pub(super) fn spawn_log_reader<R>(reader: R, stream_name: &'static str)
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

pub(super) fn parse_core_event_payload(event_name: &str, data: &str) -> CoreEventPayload {
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

pub(super) fn emit_bridge_event(app_handle: &AppHandle, event: CoreBridgeEvent) {
    let _ = app_handle.emit("core://event", event);
}

pub(super) fn abort_events_task(state: &mut CoreState) {
    if let Some(task) = state.events_task.take() {
        task.abort();
    }
}

pub(super) fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

pub(super) fn build_plugin_multipart_body(
    boundary: &str,
    payload: &[u8],
    file_name: &str,
) -> Vec<u8> {
    let safe_file_name = file_name.replace(['\r', '\n', '"'], "_");

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

pub(super) fn parse_gui_close_behavior(settings: &BTreeMap<String, String>) -> GuiCloseBehavior {
    if let Some(raw_behavior) = settings.get(SETTING_KEY_GUI_CLOSE_BEHAVIOR)
        && let Some(parsed) = GuiCloseBehavior::parse(raw_behavior)
    {
        return parsed;
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

pub(super) fn terminate_child(child: &mut Child) -> Result<()> {
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
