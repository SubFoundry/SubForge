use anyhow::Result;
use tauri::AppHandle;

use super::core_manager::CoreManager;
use super::helpers::{abort_events_task, emit_bridge_event, parse_core_event_payload};
use super::types::CoreBridgeEvent;

impl CoreManager {
    pub(crate) fn start_events_bridge(&self, app_handle: AppHandle) -> Result<()> {
        let (base_url, admin_token, core_running) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            self.try_restore_admin_token(&mut state);
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
                state.child.is_some() || state.admin_token.is_some(),
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
}
