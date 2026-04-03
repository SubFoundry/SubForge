use std::collections::{BTreeMap, BTreeSet};

use app_plugin_runtime::LoadedPlugin;
use serde_json::Value;

use crate::source_service::{PreparedConfig, SourceService};
use crate::utils::{is_scalar_json, stringify_secret_value, validate_property_value};
use crate::{CoreError, CoreResult};

impl<'a> SourceService<'a> {
    pub(super) fn validate_and_split_config(
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
}
