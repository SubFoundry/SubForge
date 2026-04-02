use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_common::{PluginType, ProxyNode, SourceInstance};
use app_plugin_runtime::{LoadedPlugin, LuaSandbox, LuaSandboxConfig, PluginRuntimeError};
use app_secrets::SecretStore;
use app_storage::{Database, SourceRepository};
use serde_json::{Map, Value};

use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult, SourceWithConfig, StaticFetcher};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScriptExecutionResult {
    pub nodes: Vec<ProxyNode>,
    pub subscription_userinfo: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum StateUpdate {
    Keep,
    Replace(Option<Value>),
}

#[derive(Debug, Clone, PartialEq)]
struct StageResult {
    state_update: StateUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptSubscription {
    Url(String),
    Content(String),
}

#[derive(Debug, Clone, PartialEq)]
struct FetchStageResult {
    state_update: StateUpdate,
    subscription: ScriptSubscription,
}

#[derive(Debug)]
pub(crate) struct ScriptExecutor<'a> {
    db: &'a Database,
    secret_store: Arc<dyn SecretStore>,
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn new(db: &'a Database, secret_store: Arc<dyn SecretStore>) -> Self {
        Self { db, secret_store }
    }

    pub(crate) async fn execute(
        &self,
        source: &SourceWithConfig,
        loaded_plugin: &LoadedPlugin,
        trigger_type: &str,
    ) -> CoreResult<ScriptExecutionResult> {
        if !matches!(loaded_plugin.manifest.plugin_type, PluginType::Script) {
            return Err(CoreError::ConfigInvalid(format!(
                "插件 {} 不是 script 类型，无法执行脚本编排",
                loaded_plugin.manifest.plugin_id
            )));
        }

        let fetch_entry = loaded_plugin
            .manifest
            .entrypoints
            .fetch
            .as_deref()
            .ok_or_else(|| script_runtime_error("script 插件缺少 fetch 入口"))?;
        let fetch_path = resolve_entrypoint_path(loaded_plugin, fetch_entry, "fetch")?;

        let sandbox_config = LuaSandboxConfig::default()
            .with_network_profile(loaded_plugin.manifest.network_profile.clone())
            .with_plugin_id(loaded_plugin.manifest.plugin_id.clone())
            .with_secret_store(Arc::clone(&self.secret_store));
        let sandbox = LuaSandbox::new_with_config(sandbox_config)?;

        let mut source_row = source.source.clone();
        let mut state = parse_persisted_state(source.source.state_json.as_deref())?;
        let fetcher = StaticFetcher::new_with_network_profile(
            self.db,
            &loaded_plugin.manifest.network_profile,
        )?;

        if state.is_none() {
            if let Some(login_entry) = loaded_plugin.manifest.entrypoints.login.as_deref() {
                let login_path = resolve_entrypoint_path(loaded_plugin, login_entry, "login")?;
                let login_result = execute_stage(
                    &sandbox,
                    "login",
                    &login_path,
                    source,
                    loaded_plugin,
                    trigger_type,
                    state.as_ref(),
                )?;
                apply_state_update(&mut state, login_result.state_update.clone());
                persist_state_if_changed(
                    self.db,
                    &mut source_row,
                    &state,
                    &login_result.state_update,
                )?;
            }
        }

        if let Some(refresh_entry) = loaded_plugin.manifest.entrypoints.refresh.as_deref() {
            let refresh_path = resolve_entrypoint_path(loaded_plugin, refresh_entry, "refresh")?;
            let refresh_result = execute_stage(
                &sandbox,
                "refresh",
                &refresh_path,
                source,
                loaded_plugin,
                trigger_type,
                state.as_ref(),
            )?;
            apply_state_update(&mut state, refresh_result.state_update.clone());
            persist_state_if_changed(
                self.db,
                &mut source_row,
                &state,
                &refresh_result.state_update,
            )?;
        }

        let fetch_result = execute_fetch_stage(
            &sandbox,
            &fetch_path,
            source,
            loaded_plugin,
            trigger_type,
            state.as_ref(),
        )?;

        apply_state_update(&mut state, fetch_result.state_update.clone());
        persist_state_if_changed(self.db, &mut source_row, &state, &fetch_result.state_update)?;

        let user_agent = source.config.get("user_agent").and_then(Value::as_str);
        let (nodes, subscription_userinfo) = match fetch_result.subscription {
            ScriptSubscription::Url(url) => {
                let result = fetcher
                    .fetch_and_cache_with_metadata(&source.source.id, &url, user_agent)
                    .await?;
                (result.nodes, result.subscription_userinfo)
            }
            ScriptSubscription::Content(content) => {
                let nodes = fetcher.parse_and_cache_content(&source.source.id, &content)?;
                (nodes, None)
            }
        };

        Ok(ScriptExecutionResult {
            nodes,
            subscription_userinfo,
        })
    }
}

fn execute_stage(
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
        let message = object
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("脚本返回失败且未提供 error");
        return Err(script_runtime_error(&format!(
            "{stage_name} 失败：{message}"
        )));
    }

    Ok(StageResult {
        state_update: parse_state_update(object)?,
    })
}

fn execute_fetch_stage(
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
        let message = object
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("脚本返回失败且未提供 error");
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

fn apply_state_update(state: &mut Option<Value>, update: StateUpdate) {
    match update {
        StateUpdate::Keep => {}
        StateUpdate::Replace(next) => *state = next,
    }
}

fn persist_state_if_changed(
    db: &Database,
    source: &mut SourceInstance,
    state: &Option<Value>,
    update: &StateUpdate,
) -> CoreResult<()> {
    if matches!(update, StateUpdate::Keep) {
        return Ok(());
    }

    source.state_json = match state {
        Some(value) => Some(serde_json::to_string(value).map_err(|error| {
            CoreError::ConfigInvalid(format!("脚本 state 序列化失败：{error}"))
        })?),
        None => None,
    };
    source.updated_at = now_rfc3339()?;
    let repository = SourceRepository::new(db);
    repository.update(source)?;
    Ok(())
}

fn parse_persisted_state(raw: Option<&str>) -> CoreResult<Option<Value>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<Value>(raw)
        .map_err(|error| CoreError::ConfigInvalid(format!("state_json 反序列化失败：{error}")))?;
    if parsed.is_null() {
        return Ok(None);
    }
    if !parsed.is_object() {
        return Err(CoreError::ConfigInvalid(
            "state_json 必须是 JSON 对象".to_string(),
        ));
    }
    Ok(Some(parsed))
}

fn resolve_entrypoint_path(
    loaded_plugin: &LoadedPlugin,
    entrypoint: &str,
    stage_name: &str,
) -> CoreResult<PathBuf> {
    let entrypoint = entrypoint.trim();
    if entrypoint.is_empty() {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口路径不能为空"
        )));
    }

    let raw_path = loaded_plugin.root_dir.join(entrypoint);
    let canonical = fs::canonicalize(&raw_path).map_err(|error| {
        script_runtime_error(&format!(
            "{stage_name} 入口脚本不存在或不可访问（{}）：{error}",
            raw_path.display()
        ))
    })?;
    if !canonical.starts_with(&loaded_plugin.root_dir) {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口脚本路径越界：{}",
            canonical.display()
        )));
    }
    if !canonical.is_file() {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口脚本不是文件：{}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

fn script_runtime_error(message: &str) -> CoreError {
    CoreError::PluginRuntime(PluginRuntimeError::ScriptRuntime(message.to_string()))
}
