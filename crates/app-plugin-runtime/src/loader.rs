use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use app_common::{ConfigSchema, PluginManifest, PluginType};
use serde_json::Value;

use crate::{PluginRuntimeError, PluginRuntimeResult};

const SUPPORTED_SPEC_MAJOR: u64 = 1;
const ALLOWED_CAPABILITIES: &[&str] = &[
    "http", "cookie", "json", "html", "base64", "secret", "log", "time",
];
const ALLOWED_NETWORK_PROFILES: &[&str] = &[
    "standard",
    "browser_chrome",
    "browser_firefox",
    "webview_assisted",
];
const ALLOWED_SCHEMA_TOP_KEYS: &[&str] = &[
    "$schema",
    "type",
    "required",
    "properties",
    "additionalProperties",
];
const ALLOWED_SCHEMA_FIELD_KEYS: &[&str] = &[
    "type",
    "title",
    "description",
    "default",
    "enum",
    "format",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "pattern",
    "x-ui",
];
const ALLOWED_SCHEMA_UI_KEYS: &[&str] = &["widget", "placeholder", "help", "group", "order"];
const ALLOWED_FIELD_TYPES: &[&str] = &["string", "number", "integer", "boolean"];

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub root_dir: PathBuf,
    pub manifest: PluginManifest,
    pub schema: ConfigSchema,
}

#[derive(Debug, Default, Clone)]
pub struct PluginLoader;

impl PluginLoader {
    pub fn new() -> Self {
        Self
    }

    pub fn load_from_dir(&self, plugin_dir: impl AsRef<Path>) -> PluginRuntimeResult<LoadedPlugin> {
        let root_dir = fs::canonicalize(plugin_dir.as_ref())?;
        let manifest_path = root_dir.join("plugin.json");
        let manifest_content = fs::read_to_string(&manifest_path)?;
        let manifest: PluginManifest =
            serde_json::from_str(&manifest_content).map_err(PluginRuntimeError::ManifestParse)?;
        self.validate_manifest(&manifest)?;

        let schema_path = root_dir.join(&manifest.config_schema);
        let schema_path = fs::canonicalize(&schema_path).map_err(|error| {
            PluginRuntimeError::Invalid(format!(
                "无法读取配置 schema 文件 {}：{}",
                manifest.config_schema, error
            ))
        })?;
        if !schema_path.starts_with(&root_dir) {
            return Err(PluginRuntimeError::Invalid(
                "config_schema 路径越界，必须位于插件目录内".to_string(),
            ));
        }

        let schema_content = fs::read_to_string(&schema_path)?;
        let schema_raw: Value =
            serde_json::from_str(&schema_content).map_err(PluginRuntimeError::SchemaParse)?;
        self.validate_schema_subset(&schema_raw)?;
        let schema: ConfigSchema =
            serde_json::from_value(schema_raw).map_err(PluginRuntimeError::SchemaParse)?;
        self.validate_schema_structure(&manifest, &schema)?;

        Ok(LoadedPlugin {
            root_dir,
            manifest,
            schema,
        })
    }

    fn validate_manifest(&self, manifest: &PluginManifest) -> PluginRuntimeResult<()> {
        if manifest.plugin_id.trim().is_empty() {
            return Err(PluginRuntimeError::Invalid(
                "plugin_id 不能为空".to_string(),
            ));
        }
        if manifest.plugin_id.contains("..")
            || manifest.plugin_id.contains('/')
            || manifest.plugin_id.contains('\\')
        {
            return Err(PluginRuntimeError::Invalid(
                "plugin_id 包含非法路径字符".to_string(),
            ));
        }
        if manifest.name.trim().is_empty() {
            return Err(PluginRuntimeError::Invalid("name 不能为空".to_string()));
        }
        if manifest.version.trim().is_empty() {
            return Err(PluginRuntimeError::Invalid("version 不能为空".to_string()));
        }
        if manifest.config_schema.trim().is_empty() {
            return Err(PluginRuntimeError::Invalid(
                "config_schema 不能为空".to_string(),
            ));
        }

        let major = parse_spec_major(&manifest.spec_version).ok_or_else(|| {
            PluginRuntimeError::Invalid(format!("spec_version 格式非法：{}", manifest.spec_version))
        })?;
        if major != SUPPORTED_SPEC_MAJOR {
            return Err(PluginRuntimeError::Incompatible(format!(
                "不支持的 spec_version：{}（当前仅支持 1.x）",
                manifest.spec_version
            )));
        }

        for capability in &manifest.capabilities {
            if !ALLOWED_CAPABILITIES.contains(&capability.as_str()) {
                return Err(PluginRuntimeError::Invalid(format!(
                    "capability 不在白名单内：{capability}"
                )));
            }
        }

        if !ALLOWED_NETWORK_PROFILES.contains(&manifest.network_profile.as_str()) {
            return Err(PluginRuntimeError::Invalid(format!(
                "network_profile 不合法：{}",
                manifest.network_profile
            )));
        }

        if matches!(manifest.plugin_type, PluginType::Script)
            && manifest
                .entrypoints
                .fetch
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(PluginRuntimeError::Invalid(
                "script 插件必须提供 entrypoints.fetch".to_string(),
            ));
        }

        Ok(())
    }

    fn validate_schema_subset(&self, schema: &Value) -> PluginRuntimeResult<()> {
        let object = schema.as_object().ok_or_else(|| {
            PluginRuntimeError::Invalid("schema 根节点必须是 JSON object".to_string())
        })?;

        for key in object.keys() {
            if !ALLOWED_SCHEMA_TOP_KEYS.contains(&key.as_str()) {
                return Err(PluginRuntimeError::Invalid(format!(
                    "schema 顶层字段不支持：{key}"
                )));
            }
        }

        if let Some(properties) = object.get("properties") {
            let properties_obj = properties.as_object().ok_or_else(|| {
                PluginRuntimeError::Invalid("schema.properties 必须是 object".to_string())
            })?;
            for (field_name, field_value) in properties_obj {
                let field_obj = field_value.as_object().ok_or_else(|| {
                    PluginRuntimeError::Invalid(format!(
                        "schema.properties.{field_name} 必须是 object"
                    ))
                })?;

                for key in field_obj.keys() {
                    if !ALLOWED_SCHEMA_FIELD_KEYS.contains(&key.as_str()) {
                        return Err(PluginRuntimeError::Invalid(format!(
                            "schema.properties.{field_name} 包含不支持字段：{key}"
                        )));
                    }
                }

                if let Some(x_ui) = field_obj.get("x-ui") {
                    let x_ui_obj = x_ui.as_object().ok_or_else(|| {
                        PluginRuntimeError::Invalid(format!(
                            "schema.properties.{field_name}.x-ui 必须是 object"
                        ))
                    })?;
                    for key in x_ui_obj.keys() {
                        if !ALLOWED_SCHEMA_UI_KEYS.contains(&key.as_str()) {
                            return Err(PluginRuntimeError::Invalid(format!(
                                "schema.properties.{field_name}.x-ui 包含不支持字段：{key}"
                            )));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_schema_structure(
        &self,
        manifest: &PluginManifest,
        schema: &ConfigSchema,
    ) -> PluginRuntimeResult<()> {
        if schema.schema_type != "object" {
            return Err(PluginRuntimeError::Invalid(
                "schema.type 必须为 object".to_string(),
            ));
        }
        if schema.properties.is_empty() {
            return Err(PluginRuntimeError::Invalid(
                "schema.properties 不能为空".to_string(),
            ));
        }

        let field_names = schema.properties.keys().cloned().collect::<HashSet<_>>();
        for required_key in &schema.required {
            if !field_names.contains(required_key) {
                return Err(PluginRuntimeError::Invalid(format!(
                    "schema.required 字段未在 properties 中定义：{required_key}"
                )));
            }
        }
        for secret_key in &manifest.secret_fields {
            if !field_names.contains(secret_key) {
                return Err(PluginRuntimeError::Invalid(format!(
                    "secret_fields 字段未在 schema.properties 中定义：{secret_key}"
                )));
            }
        }

        for (field_name, field) in &schema.properties {
            if !ALLOWED_FIELD_TYPES.contains(&field.property_type.as_str()) {
                return Err(PluginRuntimeError::Invalid(format!(
                    "schema.properties.{field_name}.type 不支持：{}",
                    field.property_type
                )));
            }
        }

        Ok(())
    }
}

fn parse_spec_major(raw: &str) -> Option<u64> {
    raw.split('.').next()?.trim().parse::<u64>().ok()
}
