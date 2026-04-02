//! app-core：业务编排层（调度、刷新、重试、状态机）。

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use app_common::{
    ConfigSchemaProperty, Plugin, ProxyNode, ProxyProtocol, ProxyTransport, SourceInstance,
    TlsConfig,
};
use app_plugin_runtime::{LoadedPlugin, PluginLoader, PluginRuntimeError};
use app_secrets::{SecretError, SecretStore};
use app_storage::{
    Database, ExportToken, ExportTokenRepository, NodeCacheRepository, PluginRepository,
    RefreshJob, RefreshJobRepository, SourceConfigRepository, SourceRepository, StorageError,
};
use base64::Engine as Base64Engine;
use base64::engine::general_purpose::{
    STANDARD as BASE64_STANDARD, STANDARD_NO_PAD as BASE64_STANDARD_NO_PAD,
    URL_SAFE as BASE64_URL_SAFE, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
};
use regex::Regex;
use reqwest::header::{CONTENT_TYPE, HeaderMap, USER_AGENT};
use reqwest::{Client as HttpClient, Url};
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SECRET_PLACEHOLDER: &str = "••••••";
const DEFAULT_SUBSCRIPTION_TIMEOUT_SEC: u64 = 30;
const MAX_SUBSCRIPTION_REDIRECTS: usize = 10;
const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("插件运行时错误：{0}")]
    PluginRuntime(#[from] PluginRuntimeError),
    #[error("存储层错误：{0}")]
    Storage(#[from] StorageError),
    #[error("密钥存储错误：{0}")]
    Secret(#[from] SecretError),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("时间格式化失败：{0}")]
    TimeFormat(#[from] time::error::Format),
    #[error("HTTP 客户端初始化失败：{0}")]
    HttpClientBuild(#[from] reqwest::Error),
    #[error("随机数生成失败：{0}")]
    Random(#[from] getrandom::Error),
    #[error("插件已安装：{0}")]
    PluginAlreadyInstalled(String),
    #[error("配置校验失败：{0}")]
    ConfigInvalid(String),
    #[error("插件不存在：{0}")]
    PluginNotFound(String),
    #[error("来源不存在：{0}")]
    SourceNotFound(String),
    #[error("订阅拉取失败：{0}")]
    SubscriptionFetch(String),
    #[error("订阅解析失败：{0}")]
    SubscriptionParse(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PluginRuntime(error) => error.code(),
            Self::ConfigInvalid(_) => "E_CONFIG_INVALID",
            Self::PluginNotFound(_) | Self::SourceNotFound(_) => "E_NOT_FOUND",
            Self::PluginAlreadyInstalled(_) => "E_PLUGIN_INVALID",
            Self::SubscriptionParse(_) => "E_PARSE",
            Self::Storage(_)
            | Self::Secret(_)
            | Self::Io(_)
            | Self::TimeFormat(_)
            | Self::Random(_)
            | Self::HttpClientBuild(_)
            | Self::SubscriptionFetch(_) => "E_INTERNAL",
        }
    }
}

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

pub trait SubscriptionParser {
    fn parse(&self, source_id: &str, payload: &str) -> CoreResult<Vec<ProxyNode>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UriListParser;

#[derive(Debug)]
pub struct StaticFetcher<'a, P: SubscriptionParser = UriListParser> {
    db: &'a Database,
    parser: P,
    client: HttpClient,
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

impl SubscriptionParser for UriListParser {
    fn parse(&self, source_id: &str, payload: &str) -> CoreResult<Vec<ProxyNode>> {
        let normalized = normalize_subscription_payload(payload);
        let updated_at = now_rfc3339()?;
        let mut nodes = Vec::new();

        for (line_number, raw_line) in normalized.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            match parse_proxy_uri_line(line, source_id, &updated_at) {
                Ok(node) => nodes.push(node),
                Err(error) => {
                    eprintln!(
                        "WARN: 解析订阅行失败（source_id={}, line={}）：{}",
                        source_id,
                        line_number + 1,
                        error
                    );
                }
            }
        }

        Ok(nodes)
    }
}

impl<'a> StaticFetcher<'a, UriListParser> {
    pub fn new(db: &'a Database) -> CoreResult<Self> {
        Self::with_parser(db, UriListParser)
    }
}

impl<'a, P> StaticFetcher<'a, P>
where
    P: SubscriptionParser,
{
    pub fn with_parser(db: &'a Database, parser: P) -> CoreResult<Self> {
        let client = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::limited(
                MAX_SUBSCRIPTION_REDIRECTS,
            ))
            .timeout(std::time::Duration::from_secs(
                DEFAULT_SUBSCRIPTION_TIMEOUT_SEC,
            ))
            .build()?;

        Ok(Self { db, parser, client })
    }

    pub async fn fetch_and_cache(
        &self,
        source_instance_id: &str,
        subscription_url: &str,
        user_agent: Option<&str>,
    ) -> CoreResult<Vec<ProxyNode>> {
        let subscription_url = subscription_url.trim();
        if subscription_url.is_empty() {
            return Err(CoreError::ConfigInvalid("订阅 URL 不能为空".to_string()));
        }

        let url = Url::parse(subscription_url)
            .map_err(|error| CoreError::ConfigInvalid(format!("订阅 URL 非法：{error}")))?;
        let mut headers = HeaderMap::new();
        if let Some(user_agent) = user_agent {
            let user_agent = user_agent.trim();
            if !user_agent.is_empty() {
                headers.insert(
                    USER_AGENT,
                    user_agent.parse().map_err(|error| {
                        CoreError::ConfigInvalid(format!("user_agent 非法：{error}"))
                    })?,
                );
            }
        }

        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|error| CoreError::SubscriptionFetch(error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(CoreError::SubscriptionFetch(format!(
                "上游响应状态码异常：{}",
                status.as_u16()
            )));
        }

        validate_content_type(response.headers())?;
        if let Some(content_length) = response.content_length() {
            if content_length > MAX_SUBSCRIPTION_BYTES as u64 {
                return Err(CoreError::SubscriptionFetch(format!(
                    "上游响应体过大：{} bytes（限制 {} bytes）",
                    content_length, MAX_SUBSCRIPTION_BYTES
                )));
            }
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|error| CoreError::SubscriptionFetch(error.to_string()))?;
        if bytes.len() > MAX_SUBSCRIPTION_BYTES {
            return Err(CoreError::SubscriptionFetch(format!(
                "上游响应体过大：{} bytes（限制 {} bytes）",
                bytes.len(),
                MAX_SUBSCRIPTION_BYTES
            )));
        }

        let payload = std::str::from_utf8(&bytes).map_err(|error| {
            CoreError::SubscriptionParse(format!("订阅内容不是 UTF-8：{error}"))
        })?;
        let nodes = self.parser.parse(source_instance_id, payload)?;

        let now = now_rfc3339()?;
        let cache_repository = NodeCacheRepository::new(self.db);
        cache_repository.upsert_nodes(source_instance_id, &nodes, &now, None)?;

        Ok(nodes)
    }
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

        let fetcher = StaticFetcher::new(self.db)?;
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

fn now_rfc3339() -> CoreResult<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn generate_secure_token() -> CoreResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(BASE64_URL_SAFE_NO_PAD.encode(bytes))
}

fn plugin_scope(plugin_id: &str) -> String {
    format!("plugin:{plugin_id}")
}

fn is_scalar_json(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

fn masked_config(
    loaded: &LoadedPlugin,
    normalized_config: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut result = normalized_config.clone();
    for key in &loaded.manifest.secret_fields {
        if result.contains_key(key) {
            result.insert(key.clone(), Value::String(SECRET_PLACEHOLDER.to_string()));
        }
    }
    result
}

fn validate_content_type(headers: &HeaderMap) -> CoreResult<()> {
    let Some(content_type) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };

    let normalized = content_type.to_ascii_lowercase();
    let allowed = normalized.starts_with("text/")
        || normalized.starts_with("application/json")
        || normalized.starts_with("application/octet-stream");

    if allowed {
        Ok(())
    } else {
        Err(CoreError::SubscriptionFetch(format!(
            "上游 Content-Type 不受支持：{content_type}"
        )))
    }
}

fn normalize_subscription_payload(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if looks_like_uri_list(trimmed) {
        return trimmed.to_string();
    }

    let compact_base64 = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if let Some(decoded) = try_decode_base64_text(&compact_base64) {
        let decoded_trimmed = decoded.trim();
        if looks_like_uri_list(decoded_trimmed) {
            return decoded_trimmed.to_string();
        }
    }

    trimmed.to_string()
}

fn looks_like_uri_list(payload: &str) -> bool {
    payload.contains("ss://")
        || payload.contains("vmess://")
        || payload.contains("vless://")
        || payload.contains("trojan://")
}

fn parse_proxy_uri_line(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    if line.starts_with("ss://") {
        return parse_ss_uri(line, source_id, updated_at);
    }
    if line.starts_with("vmess://") {
        return parse_vmess_uri(line, source_id, updated_at);
    }
    if line.starts_with("vless://") {
        return parse_vless_uri(line, source_id, updated_at);
    }
    if line.starts_with("trojan://") {
        return parse_trojan_uri(line, source_id, updated_at);
    }

    Err(CoreError::SubscriptionParse(format!(
        "不支持的 URI 协议：{line}"
    )))
}

fn parse_ss_uri(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let raw = &line["ss://".len()..];
    let (without_fragment, name) = split_fragment(raw);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);

    let (credential_part, host_part) = if let Some((cred, host)) = without_query.rsplit_once('@') {
        (cred.to_string(), host.to_string())
    } else {
        let decoded = try_decode_base64_text(without_query)
            .ok_or_else(|| CoreError::SubscriptionParse("ss URI 缺少 @server:port".to_string()))?;
        let (cred, host) = decoded
            .rsplit_once('@')
            .ok_or_else(|| CoreError::SubscriptionParse("ss URI 凭证无法解析".to_string()))?;
        (cred.to_string(), host.to_string())
    };

    let credential_decoded =
        try_decode_base64_text(&credential_part).unwrap_or_else(|| credential_part.clone());
    let (cipher, password) = credential_decoded.split_once(':').ok_or_else(|| {
        CoreError::SubscriptionParse("ss URI 凭证必须为 method:password".to_string())
    })?;
    let (server, port) = parse_host_port(&host_part)?;

    let mut extra = BTreeMap::new();
    extra.insert("cipher".to_string(), Value::String(cipher.to_string()));
    extra.insert("password".to_string(), Value::String(password.to_string()));

    Ok(build_proxy_node(
        source_id,
        name.unwrap_or_else(|| format!("ss-{server}:{port}")),
        ProxyProtocol::Ss,
        server,
        port,
        ProxyTransport::Tcp,
        TlsConfig {
            enabled: false,
            server_name: None,
        },
        extra,
        updated_at,
    ))
}

fn parse_vmess_uri(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let raw = line["vmess://".len()..].trim();
    let decoded = try_decode_base64_text(raw)
        .ok_or_else(|| CoreError::SubscriptionParse("vmess URI Base64 解码失败".to_string()))?;
    let payload = serde_json::from_str::<Value>(&decoded)
        .map_err(|error| CoreError::SubscriptionParse(format!("vmess JSON 非法：{error}")))?;

    let server = payload
        .get("add")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::SubscriptionParse("vmess 缺少 add".to_string()))?
        .to_string();
    let port = payload
        .get("port")
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|raw| u16::try_from(raw).ok())
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<u16>().ok()))
        })
        .ok_or_else(|| CoreError::SubscriptionParse("vmess 缺少有效 port".to_string()))?;
    let name = payload
        .get("ps")
        .and_then(Value::as_str)
        .unwrap_or("vmess")
        .to_string();
    let transport = match payload.get("net").and_then(Value::as_str) {
        Some("ws") => ProxyTransport::Ws,
        Some("grpc") => ProxyTransport::Grpc,
        Some("h2") => ProxyTransport::H2,
        Some("quic") => ProxyTransport::Quic,
        _ => ProxyTransport::Tcp,
    };

    let tls_enabled = matches!(
        payload.get("tls").and_then(Value::as_str),
        Some("tls" | "reality")
    );
    let server_name = payload
        .get("sni")
        .and_then(Value::as_str)
        .or_else(|| payload.get("host").and_then(Value::as_str))
        .map(ToString::to_string);

    let mut extra = BTreeMap::new();
    if let Some(uuid) = payload.get("id").and_then(Value::as_str) {
        extra.insert("uuid".to_string(), Value::String(uuid.to_string()));
    }
    if let Some(path) = payload.get("path").and_then(Value::as_str) {
        extra.insert("path".to_string(), Value::String(path.to_string()));
    }

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Vmess,
        server,
        port,
        transport,
        TlsConfig {
            enabled: tls_enabled,
            server_name,
        },
        extra,
        updated_at,
    ))
}

fn parse_vless_uri(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let url = Url::parse(line)
        .map_err(|error| CoreError::SubscriptionParse(format!("vless URI 非法：{error}")))?;
    let server = url
        .host_str()
        .ok_or_else(|| CoreError::SubscriptionParse("vless URI 缺少 host".to_string()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CoreError::SubscriptionParse("vless URI 缺少端口".to_string()))?;
    let name = url
        .fragment()
        .filter(|value| !value.is_empty())
        .unwrap_or("vless")
        .to_string();
    let transport = map_transport(url.query_pairs().find_map(|(k, v)| {
        if k == "type" {
            Some(v.to_string())
        } else {
            None
        }
    }));
    let security = url.query_pairs().find_map(|(k, v)| {
        if k == "security" {
            Some(v.to_string())
        } else {
            None
        }
    });
    let sni = url.query_pairs().find_map(|(k, v)| {
        if k == "sni" {
            Some(v.to_string())
        } else {
            None
        }
    });

    let mut extra = BTreeMap::new();
    if !url.username().is_empty() {
        extra.insert(
            "uuid".to_string(),
            Value::String(url.username().to_string()),
        );
    }

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Vless,
        server,
        port,
        transport,
        TlsConfig {
            enabled: matches!(security.as_deref(), Some("tls" | "reality" | "xtls")),
            server_name: sni,
        },
        extra,
        updated_at,
    ))
}

fn parse_trojan_uri(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let url = Url::parse(line)
        .map_err(|error| CoreError::SubscriptionParse(format!("trojan URI 非法：{error}")))?;
    let server = url
        .host_str()
        .ok_or_else(|| CoreError::SubscriptionParse("trojan URI 缺少 host".to_string()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CoreError::SubscriptionParse("trojan URI 缺少端口".to_string()))?;
    let name = url
        .fragment()
        .filter(|value| !value.is_empty())
        .unwrap_or("trojan")
        .to_string();

    let mut extra = BTreeMap::new();
    if !url.username().is_empty() {
        extra.insert(
            "password".to_string(),
            Value::String(url.username().to_string()),
        );
    }
    let sni = url.query_pairs().find_map(|(k, v)| {
        if k == "sni" {
            Some(v.to_string())
        } else {
            None
        }
    });

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Trojan,
        server,
        port,
        ProxyTransport::Tcp,
        TlsConfig {
            enabled: true,
            server_name: sni,
        },
        extra,
        updated_at,
    ))
}

fn split_fragment(raw: &str) -> (&str, Option<String>) {
    if let Some((value, fragment)) = raw.split_once('#') {
        (value, Some(fragment.to_string()))
    } else {
        (raw, None)
    }
}

fn parse_host_port(raw: &str) -> CoreResult<(String, u16)> {
    if let Some(stripped) = raw.strip_prefix('[') {
        let (host, remainder) = stripped
            .split_once(']')
            .ok_or_else(|| CoreError::SubscriptionParse(format!("host 非法：{raw}")))?;
        let port = remainder
            .strip_prefix(':')
            .ok_or_else(|| CoreError::SubscriptionParse(format!("端口缺失：{raw}")))?
            .parse::<u16>()
            .map_err(|error| CoreError::SubscriptionParse(format!("端口非法：{error}")))?;
        return Ok((host.to_string(), port));
    }

    let (host, port) = raw
        .rsplit_once(':')
        .ok_or_else(|| CoreError::SubscriptionParse(format!("host:port 解析失败：{raw}")))?;
    let port = port
        .parse::<u16>()
        .map_err(|error| CoreError::SubscriptionParse(format!("端口非法：{error}")))?;
    Ok((host.to_string(), port))
}

fn map_transport(raw: Option<String>) -> ProxyTransport {
    match raw.as_deref() {
        Some("ws") => ProxyTransport::Ws,
        Some("grpc") => ProxyTransport::Grpc,
        Some("h2") => ProxyTransport::H2,
        Some("quic") => ProxyTransport::Quic,
        _ => ProxyTransport::Tcp,
    }
}

fn build_proxy_node(
    source_id: &str,
    name: String,
    protocol: ProxyProtocol,
    server: String,
    port: u16,
    transport: ProxyTransport,
    tls: TlsConfig,
    extra: BTreeMap<String, Value>,
    updated_at: &str,
) -> ProxyNode {
    ProxyNode {
        id: build_proxy_node_id(
            source_id,
            &protocol,
            &server,
            port,
            &name,
            extra.get("uuid").or_else(|| extra.get("password")),
        ),
        name,
        protocol,
        server,
        port,
        transport,
        tls,
        extra,
        source_id: source_id.to_string(),
        tags: Vec::new(),
        region: None,
        updated_at: updated_at.to_string(),
    }
}

fn build_proxy_node_id(
    source_id: &str,
    protocol: &ProxyProtocol,
    server: &str,
    port: u16,
    name: &str,
    credential: Option<&Value>,
) -> String {
    let mut hasher = DefaultHasher::new();
    source_id.hash(&mut hasher);
    protocol.hash(&mut hasher);
    server.hash(&mut hasher);
    port.hash(&mut hasher);
    name.hash(&mut hasher);
    if let Some(value) = credential {
        value.to_string().hash(&mut hasher);
    }
    format!("node-{:016x}", hasher.finish())
}

fn try_decode_base64_text(raw: &str) -> Option<String> {
    for engine in [
        &BASE64_STANDARD,
        &BASE64_STANDARD_NO_PAD,
        &BASE64_URL_SAFE,
        &BASE64_URL_SAFE_NO_PAD,
    ] {
        if let Ok(bytes) = engine.decode(raw) {
            if let Ok(text) = String::from_utf8(bytes) {
                return Some(text);
            }
        }
    }
    None
}

fn validate_property_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    value: &Value,
) -> CoreResult<Value> {
    let mut validated = match property.property_type.as_str() {
        "string" => {
            let text = value.as_str().ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 string"))
            })?;
            if let Some(min_length) = property.min_length {
                if text.chars().count() < min_length as usize {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 长度不能小于 {min_length}"
                    )));
                }
            }
            if let Some(max_length) = property.max_length {
                if text.chars().count() > max_length as usize {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 长度不能大于 {max_length}"
                    )));
                }
            }
            if let Some(pattern) = &property.pattern {
                let regex = Regex::new(pattern).map_err(|error| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 的 pattern 非法：{error}"))
                })?;
                if !regex.is_match(text) {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不匹配 pattern 约束"
                    )));
                }
            }
            Value::String(text.to_string())
        }
        "number" => {
            let number = value.as_f64().ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 number"))
            })?;
            if let Some(minimum) = property.minimum {
                if number < minimum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能小于 {minimum}"
                    )));
                }
            }
            if let Some(maximum) = property.maximum {
                if number > maximum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能大于 {maximum}"
                    )));
                }
            }
            let parsed = serde_json::Number::from_f64(number).ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 不是有效 number"))
            })?;
            Value::Number(parsed)
        }
        "integer" => {
            let parsed = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
                .ok_or_else(|| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 integer"))
                })?;
            if let Some(minimum) = property.minimum {
                if (parsed as f64) < minimum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能小于 {minimum}"
                    )));
                }
            }
            if let Some(maximum) = property.maximum {
                if (parsed as f64) > maximum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能大于 {maximum}"
                    )));
                }
            }
            Value::Number(parsed.into())
        }
        "boolean" => Value::Bool(value.as_bool().ok_or_else(|| {
            CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 boolean"))
        })?),
        _ => {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 包含不支持类型：{}",
                property.property_type
            )));
        }
    };

    if let Some(enum_values) = &property.enum_values {
        if !enum_values.iter().any(|item| item == &validated) {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 必须在枚举值范围内"
            )));
        }
    }

    if property.property_type == "integer" {
        // 统一整数 JSON 形态，避免 1 和 1.0 在枚举比较时产生歧义。
        if let Some(raw) = validated.as_i64() {
            validated = Value::Number(raw.into());
        }
    }

    Ok(validated)
}

fn stringify_secret_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    value: &Value,
) -> CoreResult<String> {
    match property.property_type.as_str() {
        "string" => value
            .as_str()
            .map(ToString::to_string)
            .ok_or_else(|| CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 string"))),
        "number" | "integer" => Ok(value
            .as_i64()
            .map(|raw| raw.to_string())
            .or_else(|| value.as_u64().map(|raw| raw.to_string()))
            .or_else(|| value.as_f64().map(|raw| raw.to_string()))
            .ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 number/integer"))
            })?),
        "boolean" => value
            .as_bool()
            .map(|raw| raw.to_string())
            .ok_or_else(|| CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 boolean"))),
        _ => Err(CoreError::ConfigInvalid(format!(
            "字段 {field_name} 包含不支持类型：{}",
            property.property_type
        ))),
    }
}

fn inflate_typed_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    raw: &str,
) -> CoreResult<Value> {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        return validate_property_value(field_name, property, &parsed);
    }

    // 兼容旧存储格式（value 直接保存为字符串）。
    let fallback = match property.property_type.as_str() {
        "string" => Value::String(raw.to_string()),
        "number" => {
            let parsed = raw.parse::<f64>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} number 解析失败：{error}"))
            })?;
            Value::Number(serde_json::Number::from_f64(parsed).ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 不是有效 number"))
            })?)
        }
        "integer" => {
            let parsed = raw.parse::<i64>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} integer 解析失败：{error}"))
            })?;
            Value::Number(parsed.into())
        }
        "boolean" => {
            let parsed = raw.parse::<bool>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} boolean 解析失败：{error}"))
            })?;
            Value::Bool(parsed)
        }
        _ => {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 包含不支持类型：{}",
                property.property_type
            )));
        }
    };

    validate_property_value(field_name, property, &fallback)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> CoreResult<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashSet;
    use std::fs;
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use app_common::ProxyProtocol;
    use app_secrets::{MemorySecretStore, SecretStore};
    use app_storage::{
        Database, ExportTokenRepository, NodeCacheRepository, PluginRepository,
        RefreshJobRepository, SourceConfigRepository, SourceRepository,
    };
    use axum::Router;
    use axum::routing::get;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::{
        CoreError, Engine, PluginInstallService, SourceService, StaticFetcher, SubscriptionParser,
        UriListParser,
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

    fn cleanup_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
