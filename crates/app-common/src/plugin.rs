use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    Static,
    Script,
}

impl PluginType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Script => "script",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginEntrypoints {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(default)]
    pub fetch: Option<String>,
}

fn default_network_profile() -> String {
    "standard".to_string()
}

fn default_anti_bot_level() -> String {
    "low".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginManifest {
    pub plugin_id: String,
    pub spec_version: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    #[serde(default)]
    pub description: Option<String>,
    pub config_schema: String,
    #[serde(default)]
    pub secret_fields: Vec<String>,
    #[serde(default)]
    pub entrypoints: PluginEntrypoints,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_network_profile")]
    pub network_profile: String,
    #[serde(default = "default_anti_bot_level")]
    pub anti_bot_level: String,
    #[serde(default)]
    pub default_refresh_interval_sec: Option<u64>,
    #[serde(default)]
    pub min_refresh_interval_sec: Option<u64>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigSchemaUi {
    #[serde(default)]
    pub widget: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub order: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigSchemaProperty {
    #[serde(rename = "type")]
    pub property_type: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<Value>>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(rename = "minLength", default)]
    pub min_length: Option<u64>,
    #[serde(rename = "maxLength", default)]
    pub max_length: Option<u64>,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(rename = "x-ui", default)]
    pub x_ui: Option<ConfigSchemaUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigSchema {
    #[serde(rename = "$schema", default)]
    pub schema: Option<String>,
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, ConfigSchemaProperty>,
    #[serde(rename = "additionalProperties", default)]
    pub additional_properties: Option<bool>,
}
