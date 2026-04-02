//! app-common：公共模型与错误定义。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ErrorResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub spec_version: String,
    pub plugin_type: String,
    pub status: String,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceInstance {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub status: String,
    pub state_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileSource {
    pub profile_id: String,
    pub source_instance_id: String,
    pub priority: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSetting {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Ss,
    Vmess,
    Vless,
    Trojan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProxyTransport {
    Tcp,
    Ws,
    Grpc,
    H2,
    Quic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub protocol: ProxyProtocol,
    pub server: String,
    pub port: u16,
    pub transport: ProxyTransport,
    pub tls: TlsConfig,
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
    pub source_id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub region: Option<String>,
    pub updated_at: String,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_error_fields_are_serializable() {
        let err = AppError::new("E_TEST", "测试错误", false);
        let json = serde_json::to_string(&err).expect("序列化失败");
        assert!(json.contains("\"code\":\"E_TEST\""));
        assert!(json.contains("\"message\":\"测试错误\""));
        assert!(json.contains("\"retryable\":false"));
    }

    #[test]
    fn domain_models_are_serializable() {
        let plugin = Plugin {
            id: "plugin-row-1".to_string(),
            plugin_id: "vendor.example.static".to_string(),
            name: "Example Plugin".to_string(),
            version: "1.0.0".to_string(),
            spec_version: "1.0".to_string(),
            plugin_type: "static".to_string(),
            status: "enabled".to_string(),
            installed_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let source = SourceInstance {
            id: "source-1".to_string(),
            plugin_id: plugin.plugin_id.clone(),
            name: "Source A".to_string(),
            status: "healthy".to_string(),
            state_json: Some("{\"cursor\":1}".to_string()),
            created_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let profile = Profile {
            id: "profile-1".to_string(),
            name: "Default".to_string(),
            description: Some("默认聚合配置".to_string()),
            created_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let profile_source = ProfileSource {
            profile_id: profile.id.clone(),
            source_instance_id: source.id.clone(),
            priority: 10,
        };
        let setting = AppSetting {
            key: "ui.theme".to_string(),
            value: "dark".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let node = ProxyNode {
            id: "node-1".to_string(),
            name: "HK-01".to_string(),
            protocol: ProxyProtocol::Vmess,
            server: "hk.example.com".to_string(),
            port: 443,
            transport: ProxyTransport::Ws,
            tls: TlsConfig {
                enabled: true,
                server_name: Some("hk.example.com".to_string()),
            },
            extra: BTreeMap::new(),
            source_id: "source-1".to_string(),
            tags: vec!["hk".to_string()],
            region: Some("hk".to_string()),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };

        assert!(
            serde_json::from_str::<Plugin>(
                &serde_json::to_string(&plugin).expect("plugin 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<SourceInstance>(
                &serde_json::to_string(&source).expect("source 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<Profile>(
                &serde_json::to_string(&profile).expect("profile 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<ProfileSource>(
                &serde_json::to_string(&profile_source).expect("profile_source 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<AppSetting>(
                &serde_json::to_string(&setting).expect("setting 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<ProxyNode>(
                &serde_json::to_string(&node).expect("proxy_node 序列化失败")
            )
            .is_ok()
        );
    }

    #[test]
    fn plugin_manifest_is_deserializable() {
        let raw = serde_json::json!({
            "plugin_id": "vendor.example.static",
            "spec_version": "1.0",
            "name": "Static Source",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json",
            "secret_fields": [],
            "capabilities": ["http", "json"],
            "network_profile": "standard",
            "anti_bot_level": "low"
        });

        let manifest: PluginManifest = serde_json::from_value(raw).expect("manifest 反序列化失败");
        assert_eq!(manifest.plugin_type, PluginType::Static);
        assert_eq!(manifest.config_schema, "schema.json");
        assert_eq!(manifest.network_profile, "standard");
    }

    #[test]
    fn config_schema_is_deserializable() {
        let raw = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "title": "订阅地址",
                    "minLength": 1
                }
            },
            "additionalProperties": false
        });

        let schema: ConfigSchema = serde_json::from_value(raw).expect("schema 反序列化失败");
        assert_eq!(schema.schema_type, "object");
        assert!(schema.properties.contains_key("url"));
        assert_eq!(schema.required, vec!["url".to_string()]);
    }
}
