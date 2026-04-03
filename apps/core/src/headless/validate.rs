use std::sync::Arc;

use anyhow::{Context, Result};
use app_secrets::{MemorySecretStore, SecretStore};
use app_storage::Database;
use time::OffsetDateTime;

use crate::config::LoadedHeadlessConfig;
use crate::headless::apply::apply_headless_configuration;

pub(crate) fn validate_headless_configuration(loaded: &LoadedHeadlessConfig) -> Result<()> {
    let database = Database::open_in_memory().context("初始化内存数据库失败")?;
    let temp_plugins_dir = std::env::temp_dir().join(format!(
        "subforge-headless-check-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(&temp_plugins_dir)
        .with_context(|| format!("创建临时插件目录失败: {}", temp_plugins_dir.display()))?;
    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let result = apply_headless_configuration(loaded, &database, secret_store, &temp_plugins_dir);
    let _ = std::fs::remove_dir_all(&temp_plugins_dir);
    result.map(|_| ())
}
