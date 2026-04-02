//! app-core：业务编排层（调度、刷新、重试、状态机）。

mod error;
mod fetcher;
mod parser;
mod utils;

pub use error::{CoreError, CoreResult};
pub use fetcher::StaticFetcher;
pub use parser::{SubscriptionParser, UriListParser};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use app_common::{Plugin, SourceInstance};
use app_plugin_runtime::{LoadedPlugin, PluginLoader};
use app_secrets::{SecretError, SecretStore};
use app_storage::{
    Database, ExportToken, ExportTokenRepository, PluginRepository, RefreshJob,
    RefreshJobRepository, SourceConfigRepository, SourceRepository,
};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::utils::{
    copy_dir_recursive, generate_secure_token, inflate_typed_value, is_scalar_json, masked_config,
    now_rfc3339, plugin_scope, stringify_secret_value, validate_property_value,
};
#[cfg(test)]
use fetcher::{redact_headers_for_log, redact_url_for_log};

const SECRET_PLACEHOLDER: &str = "••••••";

#[derive(Debug, Clone, PartialEq)]
pub struct SourceWithConfig {
    pub source: SourceInstance,
    pub config: BTreeMap<String, Value>,
}

#[derive(Debug)]
pub struct SourceService<'a> {
    db: &'a Database,
    secret_store: &'a dyn SecretStore,
    loader: PluginLoader,
    plugins_dir: PathBuf,
}

#[derive(Debug)]
pub struct Engine<'a> {
    db: &'a Database,
    secret_store: &'a dyn SecretStore,
    plugins_dir: PathBuf,
}

#[derive(Debug)]
pub struct PluginInstallService<'a> {
    db: &'a Database,
    loader: PluginLoader,
    plugins_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshResult {
    pub refresh_job_id: String,
    pub source_id: String,
    pub node_count: usize,
}

struct PreparedConfig {
    normalized: BTreeMap<String, Value>,
    non_secret: BTreeMap<String, String>,
    secret: BTreeMap<String, String>,
}

impl<'a> SourceService<'a> {
    pub fn new(
        db: &'a Database,
        plugins_dir: impl Into<PathBuf>,
        secret_store: &'a dyn SecretStore,
    ) -> Self {
        Self {
            db,
            secret_store,
            loader: PluginLoader::new(),
            plugins_dir: plugins_dir.into(),
        }
    }

    pub fn create_source(
        &self,
        plugin_id: &str,
        name: &str,
        config: BTreeMap<String, Value>,
    ) -> CoreResult<SourceWithConfig> {
        if name.trim().is_empty() {
            return Err(CoreError::ConfigInvalid("name 不能为空".to_string()));
        }

        let loaded = self.load_installed_plugin(plugin_id)?;
        let prepared = self.validate_and_split_config(&loaded, &config)?;
        let now = now_rfc3339()?;
        let source = SourceInstance {
            id: format!(
                "source-{}-{}",
                plugin_id.replace('.', "-"),
                OffsetDateTime::now_utc().unix_timestamp_nanos()
            ),
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
            status: "healthy".to_string(),
            state_json: None,
            created_at: now.clone(),
            updated_at: now,
        };

        let source_repository = SourceRepository::new(self.db);
        let config_repository = SourceConfigRepository::new(self.db);
        source_repository.insert(&source)?;

        if let Err(error) =
            self.persist_source_config(&source, &loaded, &prepared, &config_repository, false)
        {
            let _ = source_repository.delete(&source.id);
            for key in prepared.secret.keys() {
                let _ = self
                    .secret_store
                    .delete(&plugin_scope(&source.plugin_id), key.as_str());
            }
            return Err(error);
        }

        Ok(SourceWithConfig {
            source,
            config: masked_config(&loaded, &prepared.normalized),
        })
    }

    pub fn get_source(&self, source_id: &str) -> CoreResult<Option<SourceWithConfig>> {
        let source_repository = SourceRepository::new(self.db);
        let source = match source_repository.get_by_id(source_id)? {
            Some(source) => source,
            None => return Ok(None),
        };
        let loaded = self.load_installed_plugin(&source.plugin_id)?;
        let config_repository = SourceConfigRepository::new(self.db);
        let stored = config_repository.get_all(&source.id)?;
        let masked = self.inflate_and_mask_config(&source, &loaded, &stored)?;

        Ok(Some(SourceWithConfig {
            source,
            config: masked,
        }))
    }

    pub fn list_sources(&self) -> CoreResult<Vec<SourceWithConfig>> {
        let source_repository = SourceRepository::new(self.db);
        let sources = source_repository.list()?;
        let config_repository = SourceConfigRepository::new(self.db);
        let mut result = Vec::with_capacity(sources.len());

        for source in sources {
            let loaded = self.load_installed_plugin(&source.plugin_id)?;
            let stored = config_repository.get_all(&source.id)?;
            let masked = self.inflate_and_mask_config(&source, &loaded, &stored)?;
            result.push(SourceWithConfig {
                source,
                config: masked,
            });
        }

        Ok(result)
    }

    pub fn update_source_config(
        &self,
        source_id: &str,
        config: BTreeMap<String, Value>,
    ) -> CoreResult<SourceWithConfig> {
        let source_repository = SourceRepository::new(self.db);
        let mut source = source_repository
            .get_by_id(source_id)?
            .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?;
        let loaded = self.load_installed_plugin(&source.plugin_id)?;
        let prepared = self.validate_and_split_config(&loaded, &config)?;
        let config_repository = SourceConfigRepository::new(self.db);
        let previous_non_secret = config_repository.get_all(&source.id)?;
        let scope = plugin_scope(&source.plugin_id);
        let previous_secret =
            self.snapshot_secret_values(&scope, &loaded.manifest.secret_fields)?;

        if let Err(error) =
            self.persist_source_config(&source, &loaded, &prepared, &config_repository, true)
        {
            let _ = config_repository.replace_all(&source.id, &previous_non_secret);
            let _ = self.restore_secret_values(
                &scope,
                &loaded.manifest.secret_fields,
                &previous_secret,
            );
            return Err(error);
        }

        source.updated_at = now_rfc3339()?;
        if let Err(error) = source_repository.update(&source) {
            let _ = config_repository.replace_all(&source.id, &previous_non_secret);
            let _ = self.restore_secret_values(
                &scope,
                &loaded.manifest.secret_fields,
                &previous_secret,
            );
            return Err(error.into());
        }

        Ok(SourceWithConfig {
            source,
            config: masked_config(&loaded, &prepared.normalized),
        })
    }

    pub fn delete_source(&self, source_id: &str) -> CoreResult<()> {
        let source_repository = SourceRepository::new(self.db);
        let source = source_repository
            .get_by_id(source_id)?
            .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?;
        let loaded = self.load_installed_plugin(&source.plugin_id)?;
        let scope = plugin_scope(&source.plugin_id);
        let previous_secret =
            self.snapshot_secret_values(&scope, &loaded.manifest.secret_fields)?;

        for secret_key in &loaded.manifest.secret_fields {
            if let Err(error) = self.secret_store.delete(&scope, secret_key) {
                let _ = self.restore_secret_values(
                    &scope,
                    &loaded.manifest.secret_fields,
                    &previous_secret,
                );
                return Err(error.into());
            }
        }

        if let Err(error) = source_repository.delete(source_id) {
            let _ = self.restore_secret_values(
                &scope,
                &loaded.manifest.secret_fields,
                &previous_secret,
            );
            return Err(error.into());
        }
        Ok(())
    }

    fn load_installed_plugin(&self, plugin_id: &str) -> CoreResult<LoadedPlugin> {
        let plugin_repository = PluginRepository::new(self.db);
        let plugin = plugin_repository.get_by_plugin_id(plugin_id)?;
        if plugin.is_none() {
            return Err(CoreError::PluginNotFound(plugin_id.to_string()));
        }
        let plugin_dir = self.plugins_dir.join(plugin_id);
        let loaded = self.loader.load_from_dir(plugin_dir)?;
        Ok(loaded)
    }

    fn validate_and_split_config(
        &self,
        loaded: &LoadedPlugin,
        config: &BTreeMap<String, Value>,
    ) -> CoreResult<PreparedConfig> {
        for required in &loaded.schema.required {
            if !config.contains_key(required) {
                return Err(CoreError::ConfigInvalid(format!(
                    "缺少必填字段：{required}"
                )));
            }
        }

        if loaded.schema.additional_properties != Some(true) {
            for key in config.keys() {
                if !loaded.schema.properties.contains_key(key) {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段未在 schema 中定义：{key}"
                    )));
                }
            }
        }

        let secret_fields = loaded
            .manifest
            .secret_fields
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();

        let mut normalized = BTreeMap::new();
        let mut non_secret = BTreeMap::new();
        let mut secret = BTreeMap::new();

        for (field_name, property) in &loaded.schema.properties {
            let raw_value = config
                .get(field_name)
                .cloned()
                .or_else(|| property.default.clone());
            if let Some(raw_value) = raw_value {
                let validated = validate_property_value(field_name, property, &raw_value)?;
                let serialized = serde_json::to_string(&validated).map_err(|error| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 序列化失败：{error}"))
                })?;

                if secret_fields.contains(field_name) {
                    secret.insert(
                        field_name.clone(),
                        stringify_secret_value(field_name, property, &validated)?,
                    );
                } else {
                    non_secret.insert(field_name.clone(), serialized);
                }
                normalized.insert(field_name.clone(), validated);
            }
        }

        if loaded.schema.additional_properties == Some(true) {
            for (field_name, value) in config {
                if loaded.schema.properties.contains_key(field_name) {
                    continue;
                }
                if !is_scalar_json(value) {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 仅允许 string/number/boolean"
                    )));
                }
                let serialized = serde_json::to_string(value).map_err(|error| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 序列化失败：{error}"))
                })?;
                non_secret.insert(field_name.clone(), serialized);
                normalized.insert(field_name.clone(), value.clone());
            }
        }

        Ok(PreparedConfig {
            normalized,
            non_secret,
            secret,
        })
    }

    fn persist_source_config(
        &self,
        source: &SourceInstance,
        loaded: &LoadedPlugin,
        prepared: &PreparedConfig,
        config_repository: &SourceConfigRepository<'_>,
        prune_secret: bool,
    ) -> CoreResult<()> {
        config_repository.replace_all(&source.id, &prepared.non_secret)?;

        let scope = plugin_scope(&source.plugin_id);
        for (key, value) in &prepared.secret {
            self.secret_store.set(&scope, key, value)?;
        }
        if prune_secret {
            for key in &loaded.manifest.secret_fields {
                if !prepared.secret.contains_key(key) {
                    self.secret_store.delete(&scope, key)?;
                }
            }
        }
        Ok(())
    }

    fn inflate_and_mask_config(
        &self,
        source: &SourceInstance,
        loaded: &LoadedPlugin,
        stored: &BTreeMap<String, String>,
    ) -> CoreResult<BTreeMap<String, Value>> {
        let mut config = BTreeMap::new();
        for (key, raw) in stored {
            if let Some(property) = loaded.schema.properties.get(key) {
                let value = inflate_typed_value(key, property, raw)?;
                config.insert(key.clone(), value);
            } else {
                let value = serde_json::from_str::<Value>(raw)
                    .unwrap_or_else(|_| Value::String(raw.clone()));
                config.insert(key.clone(), value);
            }
        }

        let secret_keys = self
            .secret_store
            .list_keys(&plugin_scope(&source.plugin_id))?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for key in &loaded.manifest.secret_fields {
            if secret_keys.contains(key) {
                config.insert(key.clone(), Value::String(SECRET_PLACEHOLDER.to_string()));
            }
        }
        Ok(config)
    }

    fn snapshot_secret_values(
        &self,
        scope: &str,
        secret_fields: &[String],
    ) -> CoreResult<BTreeMap<String, String>> {
        let mut snapshot = BTreeMap::new();
        for secret_key in secret_fields {
            match self.secret_store.get(scope, secret_key) {
                Ok(value) => {
                    snapshot.insert(secret_key.clone(), value.to_string());
                }
                Err(SecretError::SecretMissing(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(snapshot)
    }

    fn restore_secret_values(
        &self,
        scope: &str,
        secret_fields: &[String],
        snapshot: &BTreeMap<String, String>,
    ) -> CoreResult<()> {
        for secret_key in secret_fields {
            if let Some(value) = snapshot.get(secret_key) {
                self.secret_store.set(scope, secret_key, value)?;
            } else {
                self.secret_store.delete(scope, secret_key)?;
            }
        }
        Ok(())
    }
}

impl<'a> Engine<'a> {
    pub fn new(
        db: &'a Database,
        plugins_dir: impl Into<PathBuf>,
        secret_store: &'a dyn SecretStore,
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
        let source_service = SourceService::new(self.db, &self.plugins_dir, self.secret_store);
        let source = source_service
            .get_source(source_id)?
            .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?;
        let loaded_plugin = source_service.load_installed_plugin(&source.source.plugin_id)?;

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

        let fetcher = StaticFetcher::new_with_network_profile(
            self.db,
            &loaded_plugin.manifest.network_profile,
        )?;
        let result = fetcher
            .fetch_and_cache(source_id, &url, user_agent.as_deref())
            .await;
        match result {
            Ok(nodes) => {
                let node_count = i64::try_from(nodes.len())
                    .map_err(|_| CoreError::ConfigInvalid("节点数量超过 i64 上限".to_string()))?;
                let finished_at = now_rfc3339()?;
                refresh_repository.mark_success(&refresh_job_id, &finished_at, node_count)?;
                Ok(SourceRefreshResult {
                    refresh_job_id,
                    source_id: source_id.to_string(),
                    node_count: nodes.len(),
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

impl<'a> PluginInstallService<'a> {
    pub fn new(db: &'a Database, plugins_dir: impl Into<PathBuf>) -> Self {
        Self {
            db,
            loader: PluginLoader::new(),
            plugins_dir: plugins_dir.into(),
        }
    }

    pub fn install_from_dir(&self, source_dir: impl AsRef<Path>) -> CoreResult<Plugin> {
        let loaded = self.loader.load_from_dir(source_dir)?;
        let repository = PluginRepository::new(self.db);
        let existing_plugin = repository.get_by_plugin_id(&loaded.manifest.plugin_id)?;

        fs::create_dir_all(&self.plugins_dir)?;
        let target_dir = self.plugins_dir.join(&loaded.manifest.plugin_id);
        if let Some(existing) = existing_plugin {
            if existing.version == loaded.manifest.version {
                return Err(CoreError::PluginAlreadyInstalled(
                    loaded.manifest.plugin_id.clone(),
                ));
            }

            if target_dir.exists() {
                fs::remove_dir_all(&target_dir)?;
            }
            repository.delete(&existing.id)?;
        }

        if target_dir.exists() {
            return Err(CoreError::PluginAlreadyInstalled(
                loaded.manifest.plugin_id.clone(),
            ));
        }
        copy_dir_recursive(&loaded.root_dir, &target_dir)?;

        let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let plugin = Plugin {
            id: format!(
                "{}-{}",
                loaded.manifest.plugin_id,
                OffsetDateTime::now_utc().unix_timestamp_nanos()
            ),
            plugin_id: loaded.manifest.plugin_id,
            name: loaded.manifest.name,
            version: loaded.manifest.version,
            spec_version: loaded.manifest.spec_version,
            plugin_type: loaded.manifest.plugin_type.as_str().to_string(),
            status: "installed".to_string(),
            installed_at: now.clone(),
            updated_at: now,
        };

        if let Err(error) = repository.insert(&plugin) {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(error.into());
        }

        Ok(plugin)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashSet;
    use std::fs;
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use app_common::ProxyProtocol;
    use app_secrets::{MemorySecretStore, SecretStore};
    use app_storage::{
        Database, ExportTokenRepository, NodeCacheRepository, PluginRepository,
        RefreshJobRepository, SourceConfigRepository, SourceRepository,
    };
    use axum::Router;
    use axum::http::{HeaderMap as AxumHeaderMap, StatusCode};
    use axum::routing::get;
    use reqwest::Url;
    use reqwest::header::{
        ACCEPT, AUTHORIZATION, COOKIE, HeaderMap as ReqwestHeaderMap, HeaderValue,
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::{
        CoreError, Engine, PluginInstallService, SourceService, StaticFetcher, SubscriptionParser,
        UriListParser, redact_headers_for_log, redact_url_for_log,
    };

    const BASE64_SUBSCRIPTION_FIXTURE: &str =
        include_str!("../tests/fixtures/subscription_base64.txt");

    #[test]
    fn install_plugin_copies_files_and_inserts_database_record() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-success");
        let plugins_dir = temp_root.join("plugins");
        let service = PluginInstallService::new(&db, &plugins_dir);

        let source = builtins_static_plugin_dir();
        let installed = service
            .install_from_dir(&source)
            .expect("安装内置插件应成功");

        let target_dir = plugins_dir.join("subforge.builtin.static");
        assert!(target_dir.join("plugin.json").is_file());
        assert!(target_dir.join("schema.json").is_file());
        assert_eq!(installed.plugin_id, "subforge.builtin.static");
        assert_eq!(installed.status, "installed");

        let repository = PluginRepository::new(&db);
        let loaded = repository
            .get_by_plugin_id("subforge.builtin.static")
            .expect("查询已安装插件失败")
            .expect("数据库中应存在插件记录");
        assert_eq!(loaded.plugin_id, "subforge.builtin.static");

        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_same_plugin_twice_returns_error() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-duplicate");
        let plugins_dir = temp_root.join("plugins");
        let service = PluginInstallService::new(&db, &plugins_dir);
        let source = builtins_static_plugin_dir();

        service.install_from_dir(&source).expect("首次安装应成功");
        let duplicate_error = service
            .install_from_dir(&source)
            .expect_err("重复安装应失败");

        assert!(matches!(
            duplicate_error,
            CoreError::PluginAlreadyInstalled(_)
        ));
        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_higher_version_plugin_treats_as_upgrade() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-upgrade");
        let plugins_dir = temp_root.join("plugins");
        let upgraded_source = create_upgraded_plugin_dir(&temp_root);
        let service = PluginInstallService::new(&db, &plugins_dir);
        let source = builtins_static_plugin_dir();

        let installed_v1 = service.install_from_dir(&source).expect("首次安装应成功");
        assert_eq!(installed_v1.version, "1.0.0");

        let installed_v2 = service
            .install_from_dir(&upgraded_source)
            .expect("升级安装应成功");
        assert_eq!(installed_v2.version, "1.0.1");

        let repository = PluginRepository::new(&db);
        let loaded = repository
            .get_by_plugin_id("subforge.builtin.static")
            .expect("查询升级后插件失败")
            .expect("升级后插件记录应存在");
        assert_eq!(loaded.version, "1.0.1");

        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_invalid_plugin_keeps_target_directory_clean() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-invalid");
        let plugins_dir = temp_root.join("plugins");
        let bad_plugin_dir = create_bad_plugin_dir(&temp_root);
        let service = PluginInstallService::new(&db, &plugins_dir);

        let error = service
            .install_from_dir(&bad_plugin_dir)
            .expect_err("非法插件安装应失败");
        assert!(matches!(error, CoreError::PluginRuntime(_)));

        let entries = fs::read_dir(&plugins_dir)
            .ok()
            .into_iter()
            .flat_map(|iter| iter.filter_map(Result::ok))
            .collect::<Vec<_>>();
        assert!(entries.is_empty(), "非法插件不应留下安装目录");

        cleanup_dir(&temp_root);
    }

    #[test]
    fn create_source_routes_secret_fields_to_secret_store() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("source-create");
        let plugins_dir = temp_root.join("plugins");
        let plugin_source_dir = create_secret_static_plugin_dir(&temp_root);
        let install_service = PluginInstallService::new(&db, &plugins_dir);
        install_service
            .install_from_dir(&plugin_source_dir)
            .expect("安装带密钥字段插件应成功");

        let secret_store = MemorySecretStore::new();
        let source_service = SourceService::new(&db, &plugins_dir, &secret_store);
        let mut config = BTreeMap::new();
        config.insert(
            "url".to_string(),
            json!("https://example.com/subscription.txt"),
        );
        config.insert("token".to_string(), json!("token-value"));
        config.insert("region".to_string(), json!("sg"));

        let created = source_service
            .create_source("vendor.example.secure-static", "Secure Source", config)
            .expect("创建来源应成功");

        let config_repository = SourceConfigRepository::new(&db);
        let persisted_config = config_repository
            .get_all(&created.source.id)
            .expect("查询来源配置失败");
        assert!(persisted_config.contains_key("url"));
        assert!(persisted_config.contains_key("region"));
        assert!(!persisted_config.contains_key("token"));

        let secret = secret_store
            .get("plugin:vendor.example.secure-static", "token")
            .expect("secret 字段应进入 SecretStore");
        assert_eq!(secret.as_str(), "token-value");
        assert_eq!(
            created.config.get("token"),
            Some(&Value::String("••••••".to_string()))
        );

        let fetched = source_service
            .get_source(&created.source.id)
            .expect("读取来源应成功")
            .expect("来源应存在");
        assert_eq!(
            fetched.config.get("token"),
            Some(&Value::String("••••••".to_string()))
        );

        let listed = source_service.list_sources().expect("列出来源应成功");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].config.get("token"),
            Some(&Value::String("••••••".to_string()))
        );

        cleanup_dir(&temp_root);
    }

    #[test]
    fn source_config_validation_error_returns_e_config_invalid() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("source-invalid-config");
        let plugins_dir = temp_root.join("plugins");
        let install_service = PluginInstallService::new(&db, &plugins_dir);
        install_service
            .install_from_dir(builtins_static_plugin_dir())
            .expect("安装内置插件应成功");

        let secret_store = MemorySecretStore::new();
        let source_service = SourceService::new(&db, &plugins_dir, &secret_store);
        let error = source_service
            .create_source("subforge.builtin.static", "Broken Source", BTreeMap::new())
            .expect_err("缺少必填字段时应失败");

        assert!(matches!(error, CoreError::ConfigInvalid(_)));
        assert_eq!(error.code(), "E_CONFIG_INVALID");
        cleanup_dir(&temp_root);
    }

    #[test]
    fn delete_source_cleans_plugin_secret() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("source-delete");
        let plugins_dir = temp_root.join("plugins");
        let plugin_source_dir = create_secret_static_plugin_dir(&temp_root);
        let install_service = PluginInstallService::new(&db, &plugins_dir);
        install_service
            .install_from_dir(&plugin_source_dir)
            .expect("安装带密钥字段插件应成功");

        let secret_store = MemorySecretStore::new();
        let source_service = SourceService::new(&db, &plugins_dir, &secret_store);
        let mut config = BTreeMap::new();
        config.insert("url".to_string(), json!("https://example.com/a"));
        config.insert("token".to_string(), json!("token-a"));

        let created = source_service
            .create_source("vendor.example.secure-static", "Secure Source", config)
            .expect("创建来源应成功");
        source_service
            .delete_source(&created.source.id)
            .expect("删除来源应成功");

        let source_repository = SourceRepository::new(&db);
        assert!(
            source_repository
                .get_by_id(&created.source.id)
                .expect("查询来源失败")
                .is_none()
        );

        let error = secret_store
            .get("plugin:vendor.example.secure-static", "token")
            .expect_err("删除来源后应清理对应 secret");
        assert_eq!(error.code(), "E_SECRET_MISSING");
        cleanup_dir(&temp_root);
    }

    #[test]
    fn uri_list_parser_supports_base64_and_skips_invalid_lines() {
        let parser = UriListParser;
        let nodes = parser
            .parse("source-fixture", BASE64_SUBSCRIPTION_FIXTURE)
            .expect("解析 fixture 应成功");

        assert_eq!(nodes.len(), 3);
        let protocols = nodes
            .iter()
            .map(|node| node.protocol.clone())
            .collect::<HashSet<_>>();
        assert!(protocols.contains(&ProxyProtocol::Ss));
        assert!(protocols.contains(&ProxyProtocol::Vmess));
        assert!(protocols.contains(&ProxyProtocol::Trojan));
    }

    #[test]
    fn uri_list_parser_handles_invalid_protocol_lines_without_failing() {
        let parser = UriListParser;
        let payload = "not-uri\nvmess://invalid\nss://invalid\nvless://missing-port";
        let nodes = parser
            .parse("source-invalid", payload)
            .expect("解析过程应不中断");

        assert!(nodes.is_empty());
    }

    #[tokio::test]
    async fn static_fetcher_fetches_parses_and_persists_node_cache() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let source_repository = SourceRepository::new(&db);
        source_repository
            .insert(&sample_source("source-fetch-1", "subforge.builtin.static"))
            .expect("写入来源实例失败");

        let (url, server_task) = start_fixture_server(
            "/sub",
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;

        let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
        let nodes = fetcher
            .fetch_and_cache(
                "source-fetch-1",
                &format!("{url}/sub"),
                Some("SubForge-Test/0.1"),
            )
            .await
            .expect("拉取并缓存应成功");
        assert_eq!(nodes.len(), 3);

        let cache_repository = NodeCacheRepository::new(&db);
        let cache = cache_repository
            .get_by_source("source-fetch-1")
            .expect("读取缓存失败")
            .expect("缓存应存在");
        assert_eq!(cache.nodes.len(), 3);

        server_task.abort();
    }

    #[tokio::test]
    async fn static_fetcher_rejects_unsupported_content_type() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let source_repository = SourceRepository::new(&db);
        source_repository
            .insert(&sample_source("source-fetch-2", "subforge.builtin.static"))
            .expect("写入来源实例失败");

        let (url, server_task) =
            start_fixture_server("/sub", "plain text".to_string(), "image/png").await;

        let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
        let error = fetcher
            .fetch_and_cache("source-fetch-2", &format!("{url}/sub"), None)
            .await
            .expect_err("非法 Content-Type 应被拒绝");
        assert!(matches!(error, CoreError::SubscriptionFetch(_)));

        server_task.abort();
    }

    #[tokio::test]
    async fn static_fetcher_browser_chrome_retries_on_429_then_succeeds() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let source_repository = SourceRepository::new(&db);
        source_repository
            .insert(&sample_source("source-fetch-3", "subforge.builtin.static"))
            .expect("写入来源实例失败");

        let (url, request_count, server_task) = start_retry_fixture_server(
            "/sub",
            vec![429, 429],
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;

        let fetcher = StaticFetcher::new_with_network_profile(&db, "browser_chrome")
            .expect("初始化 browser_chrome StaticFetcher 失败");
        let started = Instant::now();
        let nodes = fetcher
            .fetch_and_cache("source-fetch-3", &format!("{url}/sub"), None)
            .await
            .expect("429 重试后应成功");

        assert_eq!(nodes.len(), 3);
        assert_eq!(request_count.load(Ordering::SeqCst), 3);
        assert!(
            started.elapsed() >= Duration::from_millis(1400),
            "退避总时长应至少接近 500ms + 1000ms"
        );

        server_task.abort();
    }

    #[test]
    fn static_fetcher_rejects_unknown_network_profile() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let error = StaticFetcher::new_with_network_profile(&db, "unknown-profile")
            .expect_err("未知网络档位必须返回错误");
        assert_eq!(error.code(), "E_CONFIG_INVALID");
    }

    #[test]
    fn request_log_redacts_sensitive_headers() {
        let mut headers = ReqwestHeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer sensitive-token"),
        );
        headers.insert(COOKIE, HeaderValue::from_static("sid=secret-cookie"));
        headers.insert(ACCEPT, HeaderValue::from_static("text/plain"));

        let redacted = redact_headers_for_log(&headers);
        assert!(redacted.contains("authorization=***"));
        assert!(redacted.contains("cookie=***"));
        assert!(redacted.contains("accept=text/plain"));
        assert!(!redacted.contains("sensitive-token"));
        assert!(!redacted.contains("secret-cookie"));
    }

    #[test]
    fn request_log_redacts_sensitive_query_parameters() {
        let original =
            Url::parse("https://example.com/subscription?token=abc123&password=pwd&region=sg")
                .expect("构建测试 URL 失败");
        let redacted = redact_url_for_log(&original);
        let parsed = Url::parse(&redacted).expect("脱敏后的 URL 应可解析");
        let query = parsed
            .query_pairs()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(query.get("token"), Some(&"***".to_string()));
        assert_eq!(query.get("password"), Some(&"***".to_string()));
        assert_eq!(query.get("region"), Some(&"sg".to_string()));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("pwd"));
    }

    #[tokio::test]
    async fn engine_refresh_source_uses_profile_headers_from_plugin_manifest() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("engine-profile-routing");
        let plugins_dir = temp_root.join("plugins");
        let install_service = PluginInstallService::new(&db, &plugins_dir);
        let standard_plugin_dir = create_static_plugin_with_network_profile(
            &temp_root,
            "standard-plugin",
            "vendor.example.profile-standard",
            "standard",
        );
        let chrome_plugin_dir = create_static_plugin_with_network_profile(
            &temp_root,
            "chrome-plugin",
            "vendor.example.profile-browser-chrome",
            "browser_chrome",
        );
        install_service
            .install_from_dir(&standard_plugin_dir)
            .expect("安装 standard 插件应成功");
        install_service
            .install_from_dir(&chrome_plugin_dir)
            .expect("安装 browser_chrome 插件应成功");

        let (url, total_requests, chrome_requests, server_task) = start_profile_gate_server(
            "/sub",
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;

        let secret_store = MemorySecretStore::new();
        let source_service = SourceService::new(&db, &plugins_dir, &secret_store);
        let mut standard_config = BTreeMap::new();
        standard_config.insert("url".to_string(), json!(format!("{url}/sub")));
        let standard_source = source_service
            .create_source(
                "vendor.example.profile-standard",
                "Standard Profile Source",
                standard_config,
            )
            .expect("创建 standard 来源应成功");

        let mut chrome_config = BTreeMap::new();
        chrome_config.insert("url".to_string(), json!(format!("{url}/sub")));
        let chrome_source = source_service
            .create_source(
                "vendor.example.profile-browser-chrome",
                "Browser Chrome Source",
                chrome_config,
            )
            .expect("创建 browser_chrome 来源应成功");

        let engine = Engine::new(&db, &plugins_dir, &secret_store);
        let standard_error = engine
            .refresh_source(&standard_source.source.id, "manual")
            .await
            .expect_err("standard 档位不应通过 Chrome Header 校验");
        assert!(matches!(standard_error, CoreError::SubscriptionFetch(_)));

        let chrome_result = engine
            .refresh_source(&chrome_source.source.id, "manual")
            .await
            .expect("browser_chrome 档位应通过 Header 校验");
        assert_eq!(chrome_result.node_count, 3);
        assert_eq!(total_requests.load(Ordering::SeqCst), 2);
        assert_eq!(chrome_requests.load(Ordering::SeqCst), 1);

        server_task.abort();
        cleanup_dir(&temp_root);
    }

    #[tokio::test]
    async fn engine_refresh_source_records_refresh_job_success() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("engine-refresh");
        let plugins_dir = temp_root.join("plugins");
        let install_service = PluginInstallService::new(&db, &plugins_dir);
        install_service
            .install_from_dir(builtins_static_plugin_dir())
            .expect("安装内置插件应成功");

        let secret_store = MemorySecretStore::new();
        let source_service = SourceService::new(&db, &plugins_dir, &secret_store);
        let (url, server_task) = start_fixture_server(
            "/sub",
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;
        let mut config = BTreeMap::new();
        config.insert("url".to_string(), json!(format!("{url}/sub")));
        let source = source_service
            .create_source("subforge.builtin.static", "Engine Source", config)
            .expect("创建来源应成功");

        let engine = Engine::new(&db, &plugins_dir, &secret_store);
        let refresh_result = engine
            .refresh_source(&source.source.id, "manual")
            .await
            .expect("刷新应成功");
        assert_eq!(refresh_result.source_id, source.source.id);
        assert_eq!(refresh_result.node_count, 3);

        let refresh_repository = RefreshJobRepository::new(&db);
        let jobs = refresh_repository
            .list_by_source(&source.source.id)
            .expect("读取 refresh_jobs 失败");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, refresh_result.refresh_job_id);
        assert_eq!(jobs[0].status, "success");
        assert_eq!(jobs[0].node_count, Some(3));
        assert!(jobs[0].error_code.is_none());

        server_task.abort();
        cleanup_dir(&temp_root);
    }

    #[test]
    fn engine_ensure_profile_export_token_is_idempotent() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("engine-token");
        let plugins_dir = temp_root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("创建插件目录失败");
        let profile_repository = app_storage::ProfileRepository::new(&db);
        let profile = app_common::Profile {
            id: "profile-engine-token".to_string(),
            name: "Engine Token".to_string(),
            description: None,
            created_at: "2026-04-02T07:00:00Z".to_string(),
            updated_at: "2026-04-02T07:00:00Z".to_string(),
        };
        profile_repository
            .insert(&profile)
            .expect("写入 profile 失败");

        let secret_store = MemorySecretStore::new();
        let engine = Engine::new(&db, &plugins_dir, &secret_store);
        let token_a = engine
            .ensure_profile_export_token(&profile.id)
            .expect("首次生成 token 应成功");
        let token_b = engine
            .ensure_profile_export_token(&profile.id)
            .expect("重复生成应返回已有 token");
        assert_eq!(token_a, token_b);
        assert_eq!(token_a.len(), 43);

        let token_repository = ExportTokenRepository::new(&db);
        let stored = token_repository
            .get_active_token(&profile.id)
            .expect("读取 active token 失败")
            .expect("应存在 active token");
        assert_eq!(stored.token, token_a);

        cleanup_dir(&temp_root);
    }

    fn builtins_static_plugin_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
    }

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间异常")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("subforge-app-core-{prefix}-{nanos}"));
        fs::create_dir_all(&path).expect("创建临时目录失败");
        path
    }

    fn create_bad_plugin_dir(base: &Path) -> PathBuf {
        let path = base.join("invalid-plugin");
        fs::create_dir_all(&path).expect("创建非法插件目录失败");
        fs::write(
            path.join("plugin.json"),
            r#"{
                "plugin_id": "vendor.example.invalid",
                "spec_version": "1.0",
                "name": "Invalid Plugin",
                "version": "1.0.0",
                "type": "static",
                "config_schema": "schema.json"
            }"#,
        )
        .expect("写入非法插件 plugin.json 失败");
        fs::write(path.join("schema.json"), r#"{"type":"object","oneOf":[]}"#)
            .expect("写入非法插件 schema.json 失败");
        path
    }

    fn create_upgraded_plugin_dir(base: &Path) -> PathBuf {
        let path = base.join("upgraded-plugin");
        fs::create_dir_all(&path).expect("创建升级插件目录失败");
        fs::copy(
            builtins_static_plugin_dir().join("schema.json"),
            path.join("schema.json"),
        )
        .expect("复制 schema.json 失败");
        let plugin_json = fs::read_to_string(builtins_static_plugin_dir().join("plugin.json"))
            .expect("读取内置 plugin.json 失败")
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.0.1\"");
        fs::write(path.join("plugin.json"), plugin_json).expect("写入升级插件 plugin.json 失败");
        path
    }

    fn create_secret_static_plugin_dir(base: &Path) -> PathBuf {
        let path = base.join("secure-static-plugin");
        fs::create_dir_all(&path).expect("创建插件目录失败");
        fs::write(
            path.join("plugin.json"),
            r#"{
                "plugin_id": "vendor.example.secure-static",
                "spec_version": "1.0",
                "name": "Secure Static Source",
                "version": "1.0.0",
                "type": "static",
                "config_schema": "schema.json",
                "secret_fields": ["token"],
                "capabilities": ["http", "json"],
                "network_profile": "standard"
            }"#,
        )
        .expect("写入 plugin.json 失败");
        fs::write(
            path.join("schema.json"),
            r#"{
                "type": "object",
                "required": ["url", "token"],
                "properties": {
                    "url": { "type": "string", "minLength": 1 },
                    "token": { "type": "string", "minLength": 1, "format": "password" },
                    "region": { "type": "string", "enum": ["auto", "hk", "sg", "us"], "default": "auto" }
                }
            }"#,
        )
        .expect("写入 schema.json 失败");
        path
    }

    fn create_static_plugin_with_network_profile(
        base: &Path,
        dir_name: &str,
        plugin_id: &str,
        network_profile: &str,
    ) -> PathBuf {
        let path = base.join(dir_name);
        fs::create_dir_all(&path).expect("创建插件目录失败");
        let plugin_json = format!(
            r#"{{
                "plugin_id": "{plugin_id}",
                "spec_version": "1.0",
                "name": "{plugin_id}",
                "version": "1.0.0",
                "type": "static",
                "config_schema": "schema.json",
                "capabilities": ["http", "json"],
                "network_profile": "{network_profile}"
            }}"#
        );
        fs::write(path.join("plugin.json"), plugin_json).expect("写入 plugin.json 失败");
        fs::write(
            path.join("schema.json"),
            r#"{
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "minLength": 1 }
                },
                "additionalProperties": false
            }"#,
        )
        .expect("写入 schema.json 失败");
        path
    }

    fn sample_source(id: &str, plugin_id: &str) -> app_common::SourceInstance {
        app_common::SourceInstance {
            id: id.to_string(),
            plugin_id: plugin_id.to_string(),
            name: format!("Source {id}"),
            status: "healthy".to_string(),
            state_json: None,
            created_at: "2026-04-02T01:10:00Z".to_string(),
            updated_at: "2026-04-02T01:10:00Z".to_string(),
        }
    }

    async fn start_fixture_server(
        route_path: &'static str,
        body: String,
        content_type: &'static str,
    ) -> (String, JoinHandle<()>) {
        let app = Router::new().route(
            route_path,
            get(move || {
                let body = body.clone();
                async move { ([(axum::http::header::CONTENT_TYPE, content_type)], body) }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("启动测试 HTTP 服务失败");
        let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
        let base_url = format!("http://{}", address);

        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试 HTTP 服务运行失败");
        });

        (base_url, task)
    }

    async fn start_profile_gate_server(
        route_path: &'static str,
        success_body: String,
        content_type: &'static str,
    ) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>, JoinHandle<()>) {
        let total_requests = Arc::new(AtomicUsize::new(0));
        let chrome_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            route_path,
            get({
                let total_requests = total_requests.clone();
                let chrome_requests = chrome_requests.clone();
                move |headers: AxumHeaderMap| {
                    let success_body = success_body.clone();
                    let total_requests = total_requests.clone();
                    let chrome_requests = chrome_requests.clone();
                    async move {
                        total_requests.fetch_add(1, Ordering::SeqCst);
                        let has_chrome_header = headers
                            .get("sec-ch-ua")
                            .and_then(|value| value.to_str().ok())
                            .map(|value| value.contains("Chromium"))
                            .unwrap_or(false);
                        let has_fetch_mode = headers
                            .get("sec-fetch-mode")
                            .and_then(|value| value.to_str().ok())
                            .map(|value| value == "navigate")
                            .unwrap_or(false);
                        if has_chrome_header && has_fetch_mode {
                            chrome_requests.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::OK,
                                [(axum::http::header::CONTENT_TYPE, content_type)],
                                success_body,
                            )
                        } else {
                            (
                                StatusCode::FORBIDDEN,
                                [(axum::http::header::CONTENT_TYPE, content_type)],
                                "missing browser_chrome fingerprint".to_string(),
                            )
                        }
                    }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("启动测试 HTTP 服务失败");
        let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
        let base_url = format!("http://{}", address);

        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试 HTTP 服务运行失败");
        });

        (base_url, total_requests, chrome_requests, task)
    }

    async fn start_retry_fixture_server(
        route_path: &'static str,
        retry_statuses: Vec<u16>,
        success_body: String,
        content_type: &'static str,
    ) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let retry_statuses = Arc::new(retry_statuses);
        let request_count = Arc::new(AtomicUsize::new(0));

        let app = Router::new().route(
            route_path,
            get({
                let retry_statuses = retry_statuses.clone();
                let request_count = request_count.clone();
                move || {
                    let retry_statuses = retry_statuses.clone();
                    let request_count = request_count.clone();
                    let success_body = success_body.clone();
                    async move {
                        let current = request_count.fetch_add(1, Ordering::SeqCst);
                        if current < retry_statuses.len() {
                            let status = StatusCode::from_u16(retry_statuses[current])
                                .expect("状态码必须合法");
                            (
                                status,
                                [(axum::http::header::CONTENT_TYPE, content_type)],
                                "retry".to_string(),
                            )
                        } else {
                            (
                                StatusCode::OK,
                                [(axum::http::header::CONTENT_TYPE, content_type)],
                                success_body,
                            )
                        }
                    }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("启动测试 HTTP 服务失败");
        let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
        let base_url = format!("http://{}", address);

        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试 HTTP 服务运行失败");
        });

        (base_url, request_count, task)
    }

    fn cleanup_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
