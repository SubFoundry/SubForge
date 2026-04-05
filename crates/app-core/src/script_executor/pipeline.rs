use std::path::Path;

use app_plugin_runtime::{LoadedPlugin, LuaSandbox};
use serde_json::{Map, Value};

use crate::script_executor::errors::script_runtime_error;
use crate::script_executor::{FetchStageResult, ScriptSubscription, StageResult, StateUpdate};
use crate::utils::now_rfc3339;
use crate::{CoreResult, SourceWithConfig};

pub(super) fn execute_stage(
    sandbox: &LuaSandbox,
    stage_name: &str,
    script_path: &Path,
    source: &SourceWithConfig,
    loaded_plugin: &LoadedPlugin,
    trigger_type: &str,
    state: Option<&Value>,
) -> CoreResult<StageResult> {
    let payload = execute_stage_entry(
        sandbox,
        stage_name,
        script_path,
        source,
        loaded_plugin,
        trigger_type,
        state,
    )?;
    let object = payload
        .as_object()
        .ok_or_else(|| script_runtime_error(&format!("{stage_name} 返回值必须是对象")))?;
    let ok = object
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| script_runtime_error(&format!("{stage_name} 返回值缺少布尔字段 ok")))?;
    if !ok {
        let message = resolve_stage_error_message(object.get("error"));
        return Err(script_runtime_error(&format!(
            "{stage_name} 失败：{message}"
        )));
    }

    Ok(StageResult {
        state_update: parse_state_update(object)?,
    })
}

pub(super) fn execute_fetch_stage(
    sandbox: &LuaSandbox,
    script_path: &Path,
    source: &SourceWithConfig,
    loaded_plugin: &LoadedPlugin,
    trigger_type: &str,
    state: Option<&Value>,
) -> CoreResult<FetchStageResult> {
    let payload = execute_stage_entry(
        sandbox,
        "fetch",
        script_path,
        source,
        loaded_plugin,
        trigger_type,
        state,
    )?;
    let object = payload
        .as_object()
        .ok_or_else(|| script_runtime_error("fetch 返回值必须是对象"))?;
    let ok = object
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| script_runtime_error("fetch 返回值缺少布尔字段 ok"))?;
    if !ok {
        let message = resolve_stage_error_message(object.get("error"));
        return Err(script_runtime_error(&format!("fetch 失败：{message}")));
    }

    let subscription = parse_subscription(object.get("subscription"))?;
    Ok(FetchStageResult {
        state_update: parse_state_update(object)?,
        subscription,
    })
}

fn execute_stage_entry(
    sandbox: &LuaSandbox,
    stage_name: &str,
    script_path: &Path,
    source: &SourceWithConfig,
    loaded_plugin: &LoadedPlugin,
    trigger_type: &str,
    state: Option<&Value>,
) -> CoreResult<Value> {
    let context = build_context(source, loaded_plugin, trigger_type, state.is_some())?;
    let config = Value::Object(
        source
            .config
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<String, Value>>(),
    );
    let state_arg = state.cloned().unwrap_or(Value::Null);
    let args = vec![context, config, state_arg];

    sandbox
        .exec_file(script_path, stage_name, &args)
        .map_err(Into::into)
}

fn build_context(
    source: &SourceWithConfig,
    loaded_plugin: &LoadedPlugin,
    trigger_type: &str,
    has_state: bool,
) -> CoreResult<Value> {
    Ok(serde_json::json!({
        "source_id": source.source.id.clone(),
        "plugin_id": loaded_plugin.manifest.plugin_id.clone(),
        "trigger_type": trigger_type,
        "has_state": has_state,
        "now": now_rfc3339()?
    }))
}

fn parse_state_update(object: &Map<String, Value>) -> CoreResult<StateUpdate> {
    match object.get("state") {
        None => Ok(StateUpdate::Keep),
        Some(value) if value.is_null() => Ok(StateUpdate::Replace(None)),
        Some(value) if value.is_object() => Ok(StateUpdate::Replace(Some(value.clone()))),
        Some(_) => Err(script_runtime_error("state 必须是对象或 null")),
    }
}

fn resolve_stage_error_message(raw_error: Option<&Value>) -> String {
    raw_error
        .and_then(format_script_error)
        .unwrap_or_else(|| "脚本返回失败且未提供 error".to_string())
}

fn format_script_error(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Object(map) => {
            let code = map
                .get("code")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|raw| !raw.is_empty());
            let message = map
                .get("message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|raw| !raw.is_empty());
            let retryable = map.get("retryable").and_then(Value::as_bool);

            let resolved = match (code, message, retryable) {
                (Some(code), Some(message), Some(retryable)) => {
                    Some(format!("{code}: {message} (retryable={retryable})"))
                }
                (Some(code), Some(message), None) => Some(format!("{code}: {message}")),
                (Some(code), None, _) => Some(code.to_string()),
                (None, Some(message), Some(retryable)) => {
                    Some(format!("{message} (retryable={retryable})"))
                }
                (None, Some(message), None) => Some(message.to_string()),
                (None, None, _) => None,
            };
            if resolved.is_some() {
                return resolved;
            }
            serde_json::to_string(value).ok().filter(|raw| raw != "{}")
        }
        Value::Array(_) | Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
        Value::Null => None,
    }
}

fn parse_subscription(raw: Option<&Value>) -> CoreResult<ScriptSubscription> {
    let object = raw
        .and_then(Value::as_object)
        .ok_or_else(|| script_runtime_error("fetch.subscription 必须是对象"))?;
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let content = object
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (url, content) {
        (Some(url), None) => Ok(ScriptSubscription::Url(url.to_string())),
        (None, Some(content)) => Ok(ScriptSubscription::Content(content.to_string())),
        (Some(_), Some(_)) => Err(script_runtime_error(
            "fetch.subscription 不能同时包含 url 和 content",
        )),
        (None, None) => Err(script_runtime_error(
            "fetch.subscription 必须提供非空 url 或 content",
        )),
    }
}
