use std::collections::{BTreeMap, BTreeSet};

use app_common::SourceInstance;
use app_plugin_runtime::LoadedPlugin;
use app_secrets::SecretError;
use serde_json::Value;

use crate::source_service::SourceService;
use crate::utils::{inflate_typed_value, plugin_scope};
use crate::{CoreError, CoreResult, SECRET_PLACEHOLDER};

impl<'a> SourceService<'a> {
    pub(super) fn inflate_source_config(
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

    pub(super) fn snapshot_secret_values(
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

    pub(super) fn apply_secret_placeholders(
        &self,
        loaded: &LoadedPlugin,
        mut config: BTreeMap<String, Value>,
        previous_secret: &BTreeMap<String, String>,
    ) -> CoreResult<BTreeMap<String, Value>> {
        for secret_key in &loaded.manifest.secret_fields {
            let is_placeholder = config
                .get(secret_key)
                .is_some_and(|value| value.as_str() == Some(SECRET_PLACEHOLDER));
            if !is_placeholder {
                continue;
            }

            let Some(secret_raw) = previous_secret.get(secret_key) else {
                return Err(CoreError::ConfigInvalid(format!(
                    "字段 {secret_key} 尚未设置，不能使用占位符"
                )));
            };
            let property = loaded.schema.properties.get(secret_key).ok_or_else(|| {
                CoreError::ConfigInvalid(format!("schema 中缺少字段：{secret_key}"))
            })?;
            let inflated = inflate_typed_value(secret_key, property, secret_raw)?;
            config.insert(secret_key.clone(), inflated);
        }

        Ok(config)
    }

    pub(super) fn restore_secret_values(
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
