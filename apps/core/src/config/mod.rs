use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use toml::Value as TomlValue;

use crate::cli::SecretBackendKind;

mod loaded;
mod utils;
mod validation;

const DEFAULT_LISTEN: &str = "127.0.0.1:18118";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_LOG_RETENTION_DAYS: u16 = 7;
const DEFAULT_REFRESH_INTERVAL_SEC: u64 = 1800;

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct HeadlessConfig {
    #[serde(default)]
    pub(crate) server: ServerSection,
    #[serde(default)]
    pub(crate) log: LogSection,
    #[serde(default)]
    pub(crate) storage: StorageSection,
    #[serde(default)]
    pub(crate) secrets: SecretsSection,
    #[serde(default)]
    pub(crate) refresh: RefreshSection,
    #[serde(default)]
    pub(crate) plugins: PluginsSection,
    #[serde(default)]
    pub(crate) sources: Vec<SourceSection>,
    #[serde(default)]
    pub(crate) profiles: Vec<ProfileSection>,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedHeadlessConfig {
    pub(crate) path: PathBuf,
    pub(crate) base_dir: PathBuf,
    pub(crate) config: HeadlessConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ServerSection {
    #[serde(default = "default_listen")]
    pub(crate) listen: String,
    #[serde(default)]
    pub(crate) admin_token: Option<String>,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            admin_token: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LogSection {
    #[serde(default = "default_log_level")]
    pub(crate) level: String,
    #[serde(default)]
    pub(crate) dir: Option<PathBuf>,
    #[serde(default = "default_log_retention_days")]
    pub(crate) retention_days: u16,
}

impl Default for LogSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            dir: None,
            retention_days: default_log_retention_days(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct StorageSection {
    #[serde(default)]
    pub(crate) db_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct SecretsSection {
    #[serde(default)]
    pub(crate) backend: Option<SecretBackend>,
    #[serde(default)]
    pub(crate) file_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecretBackend {
    #[default]
    Keyring,
    Env,
    File,
    Memory,
}

impl SecretBackend {
    pub(crate) fn to_cli_backend(&self) -> SecretBackendKind {
        match self {
            Self::Keyring => SecretBackendKind::Keyring,
            Self::Env => SecretBackendKind::Env,
            Self::File => SecretBackendKind::File,
            Self::Memory => SecretBackendKind::Memory,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RefreshSection {
    #[serde(default = "default_auto_on_start")]
    pub(crate) auto_on_start: bool,
    #[serde(default = "default_refresh_interval_sec")]
    pub(crate) default_interval_sec: u64,
}

impl Default for RefreshSection {
    fn default() -> Self {
        Self {
            auto_on_start: default_auto_on_start(),
            default_interval_sec: default_refresh_interval_sec(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct PluginsSection {
    #[serde(default)]
    pub(crate) dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct SourceSection {
    pub(crate) name: String,
    pub(crate) plugin: String,
    #[serde(default)]
    pub(crate) network_profile: Option<String>,
    #[serde(default)]
    pub(crate) refresh_interval_sec: Option<u64>,
    #[serde(default)]
    pub(crate) config: BTreeMap<String, TomlValue>,
    #[serde(default)]
    pub(crate) secrets: BTreeMap<String, SecretValueSource>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct SecretValueSource {
    #[serde(default)]
    pub(crate) env: Option<String>,
    #[serde(default)]
    pub(crate) value: Option<TomlValue>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct ProfileSection {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) sources: Vec<String>,
    #[serde(default)]
    pub(crate) export_token: Option<String>,
}

fn default_listen() -> String {
    DEFAULT_LISTEN.to_string()
}

fn default_log_level() -> String {
    DEFAULT_LOG_LEVEL.to_string()
}

const fn default_log_retention_days() -> u16 {
    DEFAULT_LOG_RETENTION_DAYS
}

const fn default_auto_on_start() -> bool {
    true
}

const fn default_refresh_interval_sec() -> u64 {
    DEFAULT_REFRESH_INTERVAL_SEC
}
