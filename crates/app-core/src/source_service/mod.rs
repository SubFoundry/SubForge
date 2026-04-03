use std::collections::BTreeMap;
use std::path::PathBuf;

use app_plugin_runtime::{LoadedPlugin, PluginLoader};
use app_secrets::SecretStore;
use app_storage::{Database, PluginRepository};
use serde_json::Value;

use crate::{CoreError, CoreResult};

mod ops;
mod secrets;
mod validation;

#[derive(Debug)]
pub struct SourceService<'a> {
    pub(super) db: &'a Database,
    pub(super) secret_store: &'a dyn SecretStore,
    pub(super) loader: PluginLoader,
    pub(super) plugins_dir: PathBuf,
}

pub(super) struct PreparedConfig {
    pub(super) normalized: BTreeMap<String, Value>,
    pub(super) non_secret: BTreeMap<String, String>,
    pub(super) secret: BTreeMap<String, String>,
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

    pub(crate) fn load_installed_plugin(&self, plugin_id: &str) -> CoreResult<LoadedPlugin> {
        let plugin_repository = PluginRepository::new(self.db);
        let plugin = plugin_repository.get_by_plugin_id(plugin_id)?;
        if plugin.is_none() {
            return Err(CoreError::PluginNotFound(plugin_id.to_string()));
        }
        let plugin_dir = self.plugins_dir.join(plugin_id);
        let loaded = self.loader.load_from_dir(plugin_dir)?;
        Ok(loaded)
    }
}
