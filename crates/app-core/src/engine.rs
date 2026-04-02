use std::path::PathBuf;
use std::sync::Arc;

use app_common::PluginType;
use app_secrets::SecretStore;
use app_storage::{Database, ExportToken, ExportTokenRepository, RefreshJob, RefreshJobRepository};
use serde_json::Value;
use time::OffsetDateTime;

use crate::script_executor::ScriptExecutor;
use crate::utils::{generate_secure_token, now_rfc3339};
use crate::{CoreError, CoreResult, SourceService, StaticFetcher};

#[derive(Debug)]
pub struct Engine<'a> {
    db: &'a Database,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshResult {
    pub refresh_job_id: String,
    pub source_id: String,
    pub node_count: usize,
    pub subscription_userinfo: Option<String>,
}

impl<'a> Engine<'a> {
    pub fn new(
        db: &'a Database,
        plugins_dir: impl Into<PathBuf>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            db,
            secret_store,
            plugins_dir: plugins_dir.into(),
        }
    }

    pub async fn refresh_source(
        &self,
        source_id: &str,
        trigger_type: &str,
    ) -> CoreResult<SourceRefreshResult> {
        let source_service =
            SourceService::new(self.db, &self.plugins_dir, self.secret_store.as_ref());
        let source = source_service
            .get_source_for_runtime(source_id)?
            .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?;
        let loaded_plugin = source_service.load_installed_plugin(&source.source.plugin_id)?;

        let refresh_job_id = format!(
            "refresh-job-{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        let refresh_repository = RefreshJobRepository::new(self.db);
        refresh_repository.insert(&RefreshJob {
            id: refresh_job_id.clone(),
            source_instance_id: source_id.to_string(),
            trigger_type: trigger_type.to_string(),
            status: "running".to_string(),
            started_at: Some(now_rfc3339()?),
            finished_at: None,
            node_count: None,
            error_code: None,
            error_message: None,
        })?;

        let result = match &loaded_plugin.manifest.plugin_type {
            PluginType::Static => {
                let url = source
                    .config
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::ConfigInvalid("来源配置缺少 url 字段".to_string()))?
                    .to_string();
                let user_agent = source
                    .config
                    .get("user_agent")
                    .and_then(Value::as_str)
                    .map(str::to_string);

                let fetcher = StaticFetcher::new_with_network_profile(
                    self.db,
                    &loaded_plugin.manifest.network_profile,
                )?;
                fetcher
                    .fetch_and_cache_with_metadata(source_id, &url, user_agent.as_deref())
                    .await
                    .map(|value| (value.nodes, value.subscription_userinfo))
            }
            PluginType::Script => {
                let script_executor = ScriptExecutor::new(self.db, Arc::clone(&self.secret_store));
                script_executor
                    .execute(&source, &loaded_plugin, trigger_type)
                    .await
                    .map(|value| (value.nodes, value.subscription_userinfo))
            }
        };
        match result {
            Ok((nodes, subscription_userinfo)) => {
                let node_count_usize = nodes.len();
                let node_count = i64::try_from(node_count_usize)
                    .map_err(|_| CoreError::ConfigInvalid("节点数量超过 i64 上限".to_string()))?;
                let finished_at = now_rfc3339()?;
                refresh_repository.mark_success(&refresh_job_id, &finished_at, node_count)?;
                Ok(SourceRefreshResult {
                    refresh_job_id,
                    source_id: source_id.to_string(),
                    node_count: node_count_usize,
                    subscription_userinfo,
                })
            }
            Err(error) => {
                let finished_at = now_rfc3339()?;
                let error_code = error.code().to_string();
                let error_message = error.to_string();
                let _ = refresh_repository.mark_failed(
                    &refresh_job_id,
                    &finished_at,
                    &error_code,
                    &error_message,
                );
                Err(error)
            }
        }
    }

    pub fn ensure_profile_export_token(&self, profile_id: &str) -> CoreResult<String> {
        let repository = ExportTokenRepository::new(self.db);
        if let Some(existing) = repository.get_active_token(profile_id)? {
            return Ok(existing.token);
        }

        let token = generate_secure_token()?;
        let created_at = now_rfc3339()?;
        repository.insert(&ExportToken {
            id: format!(
                "export-token-{}",
                OffsetDateTime::now_utc().unix_timestamp_nanos()
            ),
            profile_id: profile_id.to_string(),
            token: token.clone(),
            token_type: "primary".to_string(),
            created_at,
            expires_at: None,
        })?;
        Ok(token)
    }
}
