use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use app_common::SourceInstance;
use app_plugin_runtime::{LoadedPlugin, PluginLoader};
use app_secrets::{SecretError, SecretStore};
use app_storage::{Database, PluginRepository, SourceConfigRepository, SourceRepository};
use serde_json::Value;
use time::OffsetDateTime;

use crate::utils::{
    inflate_typed_value, is_scalar_json, masked_config, now_rfc3339, plugin_scope,
    stringify_secret_value, validate_property_value,
};
use crate::{CoreError, CoreResult, SECRET_PLACEHOLDER, SourceWithConfig};

#[derive(Debug)]
pub struct SourceService<'a> {
    db: &'a Database,
    secret_store: &'a dyn SecretStore,
    loader: PluginLoader,
    plugins_dir: PathBuf,
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
        let masked = self.inflate_source_config(&source, &loaded, &stored, true)?;

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
            let masked = self.inflate_source_config(&source, &loaded, &stored, true)?;
            result.push(SourceWithConfig {
                source,
                config: masked,
            });
        }

        Ok(result)
    }

    pub(crate) fn get_source_for_runtime(
        &self,
        source_id: &str,
    ) -> CoreResult<Option<SourceWithConfig>> {
        let source_repository = SourceRepository::new(self.db);
        let source = match source_repository.get_by_id(source_id)? {
            Some(source) => source,
            None => return Ok(None),
        };
        let loaded = self.load_installed_plugin(&source.plugin_id)?;
        let config_repository = SourceConfigRepository::new(self.db);
        let stored = config_repository.get_all(&source.id)?;
        let config = self.inflate_source_config(&source, &loaded, &stored, false)?;

        Ok(Some(SourceWithConfig { source, config }))
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

    fn inflate_source_config(
        &self,
        source: &SourceInstance,
        loaded: &LoadedPlugin,
        stored: &BTreeMap<String, String>,
        mask_secret: bool,
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

        let scope = plugin_scope(&source.plugin_id);
        let secret_keys = if mask_secret {
            Some(
                self.secret_store
                    .list_keys(&scope)?
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
            )
        } else {
            None
        };
        for key in &loaded.manifest.secret_fields {
            if mask_secret {
                if secret_keys.as_ref().is_some_and(|keys| keys.contains(key)) {
                    config.insert(key.clone(), Value::String(SECRET_PLACEHOLDER.to_string()));
                }
                continue;
            }

            let Some(property) = loaded.schema.properties.get(key) else {
                continue;
            };
            match self.secret_store.get(&scope, key) {
                Ok(secret) => {
                    let value = inflate_typed_value(key, property, secret.as_str())?;
                    config.insert(key.clone(), value);
                }
                Err(SecretError::SecretMissing(_)) => {}
                Err(error) => return Err(error.into()),
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
