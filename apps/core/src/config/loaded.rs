use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value as JsonValue;

use crate::cli::SecretBackendKind;
use crate::config::utils::{resolve_path, toml_to_json_value};
use crate::config::validation::validate_loaded_config;
use crate::config::{HeadlessConfig, LoadedHeadlessConfig, SecretValueSource, SourceSection};
use crate::security::admin_token_config_permission_warning;

impl LoadedHeadlessConfig {
    pub(crate) fn from_file(path: &Path) -> Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .context("读取当前目录失败")?
                .join(path)
        };
        let content = std::fs::read_to_string(&absolute)
            .with_context(|| format!("读取配置文件失败: {}", absolute.display()))?;
        let parsed = toml::from_str::<HeadlessConfig>(&content)
            .with_context(|| format!("TOML 解析失败: {}", absolute.display()))?;

        let base_dir = absolute
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("配置文件路径缺少父目录: {}", absolute.display()))?;
        let loaded = Self {
            path: absolute,
            base_dir,
            config: parsed,
        };
        validate_loaded_config(&loaded)?;
        if let Some(warning) = admin_token_config_permission_warning(
            &loaded.path,
            loaded.config.server.admin_token.is_some(),
        ) {
            eprintln!("{warning}");
        }
        Ok(loaded)
    }

    pub(crate) fn parse_listen_addr(&self) -> Result<SocketAddr> {
        self.config
            .server
            .listen
            .parse::<SocketAddr>()
            .with_context(|| format!("server.listen 非法: {}", self.config.server.listen))
    }

    pub(crate) fn listen_host_port(&self) -> Result<(String, u16)> {
        let addr = self.parse_listen_addr()?;
        Ok((addr.ip().to_string(), addr.port()))
    }

    pub(crate) fn resolved_db_path(&self) -> Result<Option<PathBuf>> {
        self.config
            .storage
            .db_path
            .as_ref()
            .map(|path| resolve_path(&self.base_dir, path))
            .transpose()
    }

    pub(crate) fn resolved_log_dir(&self) -> Result<Option<PathBuf>> {
        self.config
            .log
            .dir
            .as_ref()
            .map(|path| resolve_path(&self.base_dir, path))
            .transpose()
    }

    pub(crate) fn resolved_plugins_dirs(&self) -> Result<Vec<PathBuf>> {
        let dirs = if self.config.plugins.dirs.is_empty() {
            vec![PathBuf::from("./plugins")]
        } else {
            self.config.plugins.dirs.clone()
        };
        dirs.into_iter()
            .map(|dir| resolve_path(&self.base_dir, &dir))
            .collect()
    }

    pub(crate) fn resolved_secrets_file_path(&self) -> Result<Option<PathBuf>> {
        self.config
            .secrets
            .file_path
            .as_ref()
            .map(|path| resolve_path(&self.base_dir, path))
            .transpose()
    }

    pub(crate) fn backend_override(&self) -> Option<SecretBackendKind> {
        self.config
            .secrets
            .backend
            .as_ref()
            .map(crate::config::SecretBackend::to_cli_backend)
    }

    pub(crate) fn resolve_source_config(
        &self,
        source: &SourceSection,
    ) -> Result<BTreeMap<String, JsonValue>> {
        let mut result = BTreeMap::new();
        for (key, value) in &source.config {
            result.insert(key.clone(), toml_to_json_value(value)?);
        }
        for (secret_key, secret_value) in &source.secrets {
            let resolved = self.resolve_secret_value(secret_key, secret_value)?;
            result.insert(secret_key.clone(), resolved);
        }
        Ok(result)
    }

    fn resolve_secret_value(&self, key: &str, source: &SecretValueSource) -> Result<JsonValue> {
        match (&source.env, &source.value) {
            (Some(env_name), None) => {
                let raw = std::env::var(env_name).with_context(|| {
                    format!("sources.secrets.{key} 依赖环境变量 {env_name}，但当前未设置")
                })?;
                Ok(JsonValue::String(raw))
            }
            (None, Some(value)) => toml_to_json_value(value),
            (Some(_), Some(_)) => {
                bail!("sources.secrets.{key} 同时配置了 env 和 value，仅允许二选一")
            }
            (None, None) => bail!("sources.secrets.{key} 必须配置 env 或 value"),
        }
    }
}
