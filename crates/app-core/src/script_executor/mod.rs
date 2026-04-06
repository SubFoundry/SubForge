use std::collections::BTreeMap;
use std::sync::Arc;

use app_common::{PluginType, ProxyNode};
use app_plugin_runtime::{LoadedPlugin, LuaSandbox, LuaSandboxConfig};
use app_secrets::SecretStore;
use app_storage::Database;
use serde_json::Value;

use crate::script_executor::errors::script_runtime_error;
use crate::script_executor::logging::{ScriptLogCollector, persist_script_logs};
use crate::script_executor::paths::resolve_entrypoint_path;
use crate::script_executor::pipeline::{execute_fetch_stage, execute_stage};
use crate::script_executor::state::{
    apply_state_update, parse_persisted_state, persist_state_if_changed,
};
use crate::{CoreError, CoreResult, SourceWithConfig, StaticFetcher};

mod errors;
mod logging;
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
    Url {
        url: String,
        headers: BTreeMap<String, String>,
        user_agent: Option<String>,
    },
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
        refresh_job_id: &str,
    ) -> CoreResult<ScriptExecutionResult> {
        if !matches!(loaded_plugin.manifest.plugin_type, PluginType::Script) {
            return Err(CoreError::ConfigInvalid(format!(
                "插件 {} 不是 script 类型，无法执行脚本编排",
                loaded_plugin.manifest.plugin_id
            )));
        }
        let plugin_id = loaded_plugin.manifest.plugin_id.clone();
        let source_id = source.source.id.clone();
        let log_collector = Arc::new(ScriptLogCollector::default());
        let execution_result = async {
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
                .with_secret_store(Arc::clone(&self.secret_store))
                .with_log_sink(log_collector.clone());
            let sandbox = LuaSandbox::new_with_config(sandbox_config)?;

            let mut source_row = source.source.clone();
            let mut state = parse_persisted_state(source.source.state_json.as_deref())?;
            let script_fetcher = StaticFetcher::new_with_network_profile(
                self.db,
                &loaded_plugin.manifest.network_profile,
            )?;
            let subscription_fetcher = StaticFetcher::new(self.db)?;

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
                persist_state_if_changed(
                    self.db,
                    &mut source_row,
                    &state,
                    &login_result.state_update,
                )?;
            }

            if let Some(refresh_entry) = loaded_plugin.manifest.entrypoints.refresh.as_deref() {
                let refresh_path =
                    resolve_entrypoint_path(loaded_plugin, refresh_entry, "refresh")?;
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

            let (nodes, subscription_userinfo) = match fetch_result.subscription {
                ScriptSubscription::Url {
                    url,
                    headers,
                    user_agent,
                } => {
                    let source_user_agent = source.config.get("user_agent").and_then(Value::as_str);
                    let effective_user_agent = user_agent.as_deref().or(source_user_agent);
                    let result = subscription_fetcher
                        .fetch_and_cache_with_metadata_and_headers(
                            &source.source.id,
                            &url,
                            effective_user_agent,
                            Some(&headers),
                        )
                        .await?;
                    (result.nodes, result.subscription_userinfo)
                }
                ScriptSubscription::Content(content) => {
                    let nodes =
                        script_fetcher.parse_and_cache_content(&source.source.id, &content)?;
                    (nodes, None)
                }
            };

            Ok(ScriptExecutionResult {
                nodes,
                subscription_userinfo,
            })
        }
        .await;

        let captured_logs = log_collector.take();
        if let Err(error) = persist_script_logs(
            self.db,
            refresh_job_id,
            &source_id,
            &plugin_id,
            captured_logs,
        ) {
            eprintln!(
                "WARN: 脚本日志持久化失败 refresh_job_id={} source_id={} plugin_id={} error={}",
                refresh_job_id, source_id, plugin_id, error
            );
        }

        execution_result
    }
}
