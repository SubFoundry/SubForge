use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(default)]
    pub routing_template_source_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClashRoutingTemplate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_config_yaml: Option<String>,
    pub groups: Vec<ClashRoutingTemplateGroup>,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub preserve_original_proxy_names: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingTemplateSourceKernel {
    Clash,
    SingBox,
    Xray,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingTemplateIr {
    pub groups: Vec<RoutingTemplateGroupIr>,
    #[serde(default)]
    pub rules: Vec<String>,
    pub source_kernel: RoutingTemplateSourceKernel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl RoutingTemplateIr {
    pub fn into_clash_template(self) -> ClashRoutingTemplate {
        let preserve_original_proxy_names =
            !matches!(self.source_kernel, RoutingTemplateSourceKernel::Unknown);
        ClashRoutingTemplate {
            base_config_yaml: None,
            groups: self
                .groups
                .into_iter()
                .map(ClashRoutingTemplateGroup::from)
                .collect(),
            rules: self.rules,
            preserve_original_proxy_names,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingTemplateGroupIr {
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: String,
    pub proxies: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub interval: Option<u32>,
    #[serde(default)]
    pub tolerance: Option<u16>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_all: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_provider: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_filter: Option<String>,
}

impl From<RoutingTemplateGroupIr> for ClashRoutingTemplateGroup {
    fn from(value: RoutingTemplateGroupIr) -> Self {
        Self {
            name: value.name,
            group_type: value.group_type,
            proxies: value.proxies,
            url: value.url,
            interval: value.interval,
            tolerance: value.tolerance,
            include_all: value.include_all,
            use_provider: value.use_provider,
            filter: value.filter,
            exclude_filter: value.exclude_filter,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClashRoutingTemplateGroup {
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: String,
    pub proxies: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub interval: Option<u32>,
    #[serde(default)]
    pub tolerance: Option<u16>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_all: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_provider: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_filter: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
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
    Hysteria2,
    Tuic,
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
