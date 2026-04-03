use std::sync::Arc;

use app_common::{PluginType, ProxyNode};
use app_plugin_runtime::{LoadedPlugin, LuaSandbox, LuaSandboxConfig};
use app_secrets::SecretStore;
use app_storage::Database;
use serde_json::Value;

use crate::script_executor::errors::script_runtime_error;
use crate::script_executor::paths::resolve_entrypoint_path;
use crate::script_executor::pipeline::{execute_fetch_stage, execute_stage};
use crate::script_executor::state::{
    apply_state_update, parse_persisted_state, persist_state_if_changed,
};
use crate::{CoreError, CoreResult, SourceWithConfig, StaticFetcher};

mod errors;
mod paths;
mod pipeline;
mod state;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScriptExecutionResult {
    pub nodes: Vec<ProxyNode>,
    pub subscription_userinfo: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum StateUpdate {
    Keep,
    Replace(Option<Value>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct StageResult {
    pub(super) state_update: StateUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ScriptSubscription {
    Url(String),
    Content(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FetchStageResult {
    pub(super) state_update: StateUpdate,
    pub(super) subscription: ScriptSubscription,
}

#[derive(Debug)]
pub(crate) struct ScriptExecutor<'a> {
    pub(super) db: &'a Database,
    pub(super) secret_store: Arc<dyn SecretStore>,
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

        if state.is_none()
            && let Some(login_entry) = loaded_plugin.manifest.entrypoints.login.as_deref()
        {
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
            persist_state_if_changed(self.db, &mut source_row, &state, &login_result.state_update)?;
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
