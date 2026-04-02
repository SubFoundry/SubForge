//! app-transform：订阅输出格式转换（clash/sing-box/base64/raw）。

use std::collections::{BTreeMap, BTreeSet};

use app_common::{Profile, ProxyNode, ProxyProtocol, ProxyTransport};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub type TransformResult<T> = Result<T, TransformError>;

/// 统一转换器接口。
pub trait Transformer {
    fn transform(&self, nodes: &[ProxyNode], profile: &Profile) -> TransformResult<String>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransformError {
    #[error("节点 `{node_name}` 缺少必填字段 `{field}`")]
    MissingField {
        node_name: String,
        field: &'static str,
    },
    #[error("YAML 序列化失败：{0}")]
    SerializeYaml(String),
    #[error("JSON 序列化失败：{0}")]
    SerializeJson(String),
}

impl TransformError {
    pub fn code(&self) -> &'static str {
        "E_TRANSFORM"
    }
}

impl From<serde_yaml::Error> for TransformError {
    fn from(error: serde_yaml::Error) -> Self {
        Self::SerializeYaml(error.to_string())
    }
}

impl From<serde_json::Error> for TransformError {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializeJson(error.to_string())
    }
}

/// Clash/Mihomo YAML 转换器。
#[derive(Debug, Clone)]
pub struct ClashTransformer {
    auto_test_url: String,
    auto_test_interval_seconds: u32,
    auto_test_tolerance: u16,
}

impl Default for ClashTransformer {
    fn default() -> Self {
        Self {
            auto_test_url: "http://www.gstatic.com/generate_204".to_string(),
            auto_test_interval_seconds: 300,
            auto_test_tolerance: 50,
        }
    }
}

impl Transformer for ClashTransformer {
    fn transform(&self, nodes: &[ProxyNode], _profile: &Profile) -> TransformResult<String> {
        let mut proxies = Vec::with_capacity(nodes.len());
        for node in nodes {
            proxies.push(build_clash_proxy(node)?);
        }

        let config = ClashConfig {
            proxies,
            proxy_groups: self.build_proxy_groups(nodes),
        };
        Ok(serde_yaml::to_string(&config)?)
    }
}

impl ClashTransformer {
    fn build_proxy_groups(&self, nodes: &[ProxyNode]) -> Vec<ClashProxyGroup> {
        let node_names = nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let region_groups = collect_region_groups(nodes);

        let mut select_proxies = Vec::new();
        push_unique_proxy_name(&mut select_proxies, "Auto");
        for region_name in region_groups.keys() {
            push_unique_proxy_name(&mut select_proxies, region_name);
        }
        for node_name in &node_names {
            push_unique_proxy_name(&mut select_proxies, node_name);
        }

        let mut groups = vec![
            ClashProxyGroup {
                name: "Select".to_string(),
                group_type: "select".to_string(),
                proxies: select_proxies,
                url: None,
                interval: None,
                tolerance: None,
            },
            ClashProxyGroup {
                name: "Auto".to_string(),
                group_type: "url-test".to_string(),
                proxies: node_names,
                url: Some(self.auto_test_url.clone()),
                interval: Some(self.auto_test_interval_seconds),
                tolerance: Some(self.auto_test_tolerance),
            },
        ];

        for (region_name, region_node_names) in region_groups {
            groups.push(ClashProxyGroup {
                name: region_name,
                group_type: "select".to_string(),
                proxies: region_node_names,
                url: None,
                interval: None,
                tolerance: None,
            });
        }

        groups
    }
}

/// sing-box JSON 转换器。
#[derive(Debug, Clone)]
pub struct SingboxTransformer {
    auto_test_url: String,
    auto_test_interval: String,
    auto_test_tolerance: u16,
}

impl Default for SingboxTransformer {
    fn default() -> Self {
        Self {
            auto_test_url: "https://www.gstatic.com/generate_204".to_string(),
            auto_test_interval: "5m".to_string(),
            auto_test_tolerance: 50,
        }
    }
}

impl Transformer for SingboxTransformer {
    fn transform(&self, nodes: &[ProxyNode], _profile: &Profile) -> TransformResult<String> {
        let mut node_tags = Vec::with_capacity(nodes.len());
        let mut outbounds = Vec::with_capacity(nodes.len() + 2);

        for node in nodes {
            node_tags.push(node.name.clone());
            outbounds.push(build_singbox_node_outbound(node)?);
        }

        let mut selector_targets = Vec::with_capacity(node_tags.len() + 1);
        push_unique_proxy_name(&mut selector_targets, "auto");
        for tag in &node_tags {
            push_unique_proxy_name(&mut selector_targets, tag);
        }

        outbounds.insert(
            0,
            SingboxOutbound {
                outbound_type: "urltest".to_string(),
                tag: "auto".to_string(),
                outbounds: Some(node_tags),
                default: None,
                url: Some(self.auto_test_url.clone()),
                interval: Some(self.auto_test_interval.clone()),
                tolerance: Some(self.auto_test_tolerance),
                server: None,
                server_port: None,
                method: None,
                password: None,
                uuid: None,
                security: None,
                alter_id: None,
                flow: None,
                network: None,
                tls: None,
                transport: None,
                obfs: None,
                congestion_control: None,
                udp_relay_mode: None,
            },
        );

        outbounds.insert(
            0,
            SingboxOutbound {
                outbound_type: "selector".to_string(),
                tag: "select".to_string(),
                outbounds: Some(selector_targets),
                default: Some("auto".to_string()),
                url: None,
                interval: None,
                tolerance: None,
                server: None,
                server_port: None,
                method: None,
                password: None,
                uuid: None,
                security: None,
                alter_id: None,
                flow: None,
                network: None,
                tls: None,
                transport: None,
                obfs: None,
                congestion_control: None,
                udp_relay_mode: None,
            },
        );

        let config = SingboxConfig { outbounds };
        Ok(serde_json::to_string_pretty(&config)?)
    }
}

fn build_singbox_node_outbound(node: &ProxyNode) -> TransformResult<SingboxOutbound> {
    let tls = build_singbox_tls(node);
    let transport = build_singbox_transport(node);

    let mut outbound = SingboxOutbound {
        outbound_type: String::new(),
        tag: node.name.clone(),
        outbounds: None,
        default: None,
        url: None,
        interval: None,
        tolerance: None,
        server: Some(node.server.clone()),
        server_port: Some(node.port),
        method: None,
        password: None,
        uuid: None,
        security: None,
        alter_id: None,
        flow: None,
        network: None,
        tls,
        transport: None,
        obfs: None,
        congestion_control: None,
        udp_relay_mode: None,
    };

    match node.protocol {
        ProxyProtocol::Ss => {
            outbound.outbound_type = "shadowsocks".to_string();
            outbound.method = Some(required_string(node, "cipher")?);
            outbound.password = Some(required_string(node, "password")?);
            outbound.tls = None;
            outbound.transport = None;
        }
        ProxyProtocol::Vmess => {
            outbound.outbound_type = "vmess".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.security = optional_string(node, "security")
                .or_else(|| optional_string(node, "cipher"))
                .or(Some("auto".to_string()));
            outbound.alter_id = optional_u32(node, "alter_id").or(Some(0));
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Vless => {
            outbound.outbound_type = "vless".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.flow = optional_string(node, "flow");
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Trojan => {
            outbound.outbound_type = "trojan".to_string();
            outbound.password = Some(required_string(node, "password")?);
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Hysteria2 => {
            outbound.outbound_type = "hysteria2".to_string();
            outbound.password = Some(
                optional_string(node, "password")
                    .or_else(|| optional_string(node, "auth"))
                    .ok_or_else(|| TransformError::MissingField {
                        node_name: node.name.clone(),
                        field: "password/auth",
                    })?,
            );
            if let Some(obfs_type) = optional_string(node, "obfs") {
                outbound.obfs = Some(SingboxObfs {
                    obfs_type,
                    password: optional_string(node, "obfs_password"),
                });
            }
            outbound.transport = None;
        }
        ProxyProtocol::Tuic => {
            outbound.outbound_type = "tuic".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.password = Some(required_string(node, "password")?);
            outbound.congestion_control = optional_string(node, "congestion_control");
            outbound.udp_relay_mode = optional_string(node, "udp_relay_mode");
            outbound.network = Some("tcp".to_string());
            outbound.transport = None;
        }
    }

    Ok(outbound)
}

fn build_singbox_tls(node: &ProxyNode) -> Option<SingboxTls> {
    let server_name = node
        .tls
        .server_name
        .clone()
        .or_else(|| optional_string(node, "sni"));
    let insecure = optional_bool(node, "skip_cert_verify");
    let alpn = optional_string_list(node, "alpn");
    let has_fields =
        server_name.is_some() || insecure.is_some() || alpn.is_some() || node.tls.enabled;
    if !has_fields {
        return None;
    }

    Some(SingboxTls {
        enabled: node.tls.enabled,
        server_name,
        insecure,
        alpn,
    })
}

fn build_singbox_transport(node: &ProxyNode) -> Option<SingboxTransport> {
    match node.transport {
        ProxyTransport::Tcp => None,
        ProxyTransport::Ws => {
            let mut headers = BTreeMap::new();
            if let Some(host) = optional_string(node, "host") {
                headers.insert("Host".to_string(), host);
            }
            Some(SingboxTransport {
                transport_type: "ws".to_string(),
                path: optional_string(node, "path"),
                headers: (!headers.is_empty()).then_some(headers),
                host: None,
                service_name: None,
                max_early_data: optional_u32(node, "max_early_data"),
                early_data_header_name: optional_string(node, "early_data_header_name"),
            })
        }
        ProxyTransport::Grpc => Some(SingboxTransport {
            transport_type: "grpc".to_string(),
            path: None,
            headers: None,
            host: None,
            service_name: optional_string(node, "grpc_service_name")
                .or_else(|| optional_string(node, "service_name")),
            max_early_data: None,
            early_data_header_name: None,
        }),
        ProxyTransport::H2 => Some(SingboxTransport {
            transport_type: "http".to_string(),
            path: optional_string(node, "path"),
            headers: None,
            host: optional_string_list(node, "host"),
            service_name: None,
            max_early_data: None,
            early_data_header_name: None,
        }),
        ProxyTransport::Quic => Some(SingboxTransport {
            transport_type: "quic".to_string(),
            path: None,
            headers: None,
            host: None,
            service_name: None,
            max_early_data: None,
            early_data_header_name: None,
        }),
    }
}

fn build_clash_proxy(node: &ProxyNode) -> TransformResult<ClashProxy> {
    let network = Some(clash_network(&node.transport).to_string());
    let ws_opts = build_ws_options(node);
    let grpc_opts = build_grpc_options(node);
    let h2_opts = build_h2_options(node);
    let tls_enabled = Some(node.tls.enabled);
    let servername = node.tls.server_name.clone();
    let sni = node
        .tls
        .server_name
        .clone()
        .or_else(|| optional_string(node, "sni"));
    let skip_cert_verify = optional_bool(node, "skip_cert_verify");

    let mut proxy = ClashProxy {
        name: node.name.clone(),
        proxy_type: String::new(),
        server: node.server.clone(),
        port: node.port,
        cipher: None,
        password: None,
        uuid: None,
        alter_id: None,
        udp: Some(true),
        tls: tls_enabled,
        sni,
        servername,
        network: None,
        flow: None,
        skip_cert_verify,
        client_fingerprint: optional_string(node, "client_fingerprint"),
        ws_opts,
        grpc_opts,
        h2_opts,
        alpn: optional_string_list(node, "alpn"),
        obfs: optional_string(node, "obfs"),
        obfs_password: optional_string(node, "obfs_password"),
        congestion_control: optional_string(node, "congestion_control"),
        udp_relay_mode: optional_string(node, "udp_relay_mode"),
    };

    match node.protocol {
        ProxyProtocol::Ss => {
            proxy.proxy_type = "ss".to_string();
            proxy.cipher = Some(required_string(node, "cipher")?);
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = Some("tcp".to_string());
            proxy.tls = None;
            proxy.sni = None;
            proxy.servername = None;
            proxy.ws_opts = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.skip_cert_verify = None;
            proxy.client_fingerprint = None;
            proxy.alpn = None;
            proxy.obfs = None;
            proxy.obfs_password = None;
            proxy.congestion_control = None;
            proxy.udp_relay_mode = None;
        }
        ProxyProtocol::Vmess => {
            proxy.proxy_type = "vmess".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.alter_id = optional_u32(node, "alter_id").or(Some(0));
            proxy.cipher = optional_string(node, "cipher").or(Some("auto".to_string()));
            proxy.network = network;
            proxy.flow = None;
            proxy.sni = None;
        }
        ProxyProtocol::Vless => {
            proxy.proxy_type = "vless".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.network = network;
            proxy.flow = optional_string(node, "flow");
            proxy.sni = None;
            proxy.alter_id = None;
            proxy.cipher = None;
        }
        ProxyProtocol::Trojan => {
            proxy.proxy_type = "trojan".to_string();
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = network;
            proxy.sni = proxy.servername.clone();
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.uuid = None;
            proxy.flow = None;
        }
        ProxyProtocol::Hysteria2 => {
            proxy.proxy_type = "hysteria2".to_string();
            proxy.password = Some(
                optional_string(node, "password")
                    .or_else(|| optional_string(node, "auth"))
                    .ok_or_else(|| TransformError::MissingField {
                        node_name: node.name.clone(),
                        field: "password/auth",
                    })?,
            );
            proxy.network = None;
            proxy.uuid = None;
            proxy.flow = None;
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.ws_opts = None;
        }
        ProxyProtocol::Tuic => {
            proxy.proxy_type = "tuic".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = None;
            proxy.flow = None;
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.ws_opts = None;
        }
    }

    Ok(proxy)
}

fn build_ws_options(node: &ProxyNode) -> Option<ClashWsOptions> {
    if !matches!(node.transport, ProxyTransport::Ws) {
        return None;
    }

    let mut headers = BTreeMap::new();
    if let Some(host) = optional_string(node, "host") {
        headers.insert("Host".to_string(), host);
    }

    Some(ClashWsOptions {
        path: optional_string(node, "path").unwrap_or_else(|| "/".to_string()),
        headers: (!headers.is_empty()).then_some(headers),
        max_early_data: optional_u32(node, "max_early_data"),
        early_data_header_name: optional_string(node, "early_data_header_name"),
    })
}

fn build_grpc_options(node: &ProxyNode) -> Option<ClashGrpcOptions> {
    if !matches!(node.transport, ProxyTransport::Grpc) {
        return None;
    }

    Some(ClashGrpcOptions {
        grpc_service_name: optional_string(node, "grpc_service_name")
            .or_else(|| optional_string(node, "service_name"))
            .unwrap_or_else(|| "grpc".to_string()),
    })
}

fn build_h2_options(node: &ProxyNode) -> Option<ClashH2Options> {
    if !matches!(node.transport, ProxyTransport::H2) {
        return None;
    }

    let host = optional_string_list(node, "host");
    Some(ClashH2Options {
        host,
        path: optional_string(node, "path"),
    })
}

fn collect_region_groups(nodes: &[ProxyNode]) -> BTreeMap<String, Vec<String>> {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    for node in nodes {
        let Some(region_name) = normalize_region_name(node.region.as_deref()) else {
            continue;
        };
        groups
            .entry(region_name)
            .or_default()
            .insert(node.name.clone());
    }

    groups
        .into_iter()
        .map(|(name, values)| (name, values.into_iter().collect::<Vec<_>>()))
        .collect()
}

fn normalize_region_name(region: Option<&str>) -> Option<String> {
    let value = region?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_ascii_uppercase())
    }
}

fn push_unique_proxy_name(target: &mut Vec<String>, name: &str) {
    if !target.iter().any(|current| current == name) {
        target.push(name.to_string());
    }
}

fn clash_network(transport: &ProxyTransport) -> &'static str {
    match transport {
        ProxyTransport::Tcp => "tcp",
        ProxyTransport::Ws => "ws",
        ProxyTransport::Grpc => "grpc",
        ProxyTransport::H2 => "h2",
        ProxyTransport::Quic => "quic",
    }
}

fn required_string(node: &ProxyNode, field: &'static str) -> TransformResult<String> {
    optional_string(node, field).ok_or_else(|| TransformError::MissingField {
        node_name: node.name.clone(),
        field,
    })
}

fn optional_string(node: &ProxyNode, field: &str) -> Option<String> {
    let value = node.extra.get(field)?;
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn optional_bool(node: &ProxyNode, field: &str) -> Option<bool> {
    let value = node.extra.get(field)?;
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(raw) if raw.eq_ignore_ascii_case("true") => Some(true),
        Value::String(raw) if raw.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn optional_u32(node: &ProxyNode, field: &str) -> Option<u32> {
    let value = node.extra.get(field)?;
    match value {
        Value::Number(number) => number.as_u64().and_then(|raw| u32::try_from(raw).ok()),
        Value::String(raw) => raw.parse::<u32>().ok(),
        _ => None,
    }
}

fn optional_string_list(node: &ProxyNode, field: &str) -> Option<Vec<String>> {
    let value = node.extra.get(field)?;
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(vec![trimmed.to_string()])
            }
        }
        Value::Array(items) => {
            let mut values = Vec::new();
            for item in items {
                if let Value::String(raw) = item {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        values.push(trimmed.to_string());
                    }
                }
            }
            (!values.is_empty()).then_some(values)
        }
        _ => None,
    }
}

#[derive(Debug, Serialize)]
struct ClashConfig {
    proxies: Vec<ClashProxy>,
    #[serde(rename = "proxy-groups")]
    proxy_groups: Vec<ClashProxyGroup>,
}

#[derive(Debug, Serialize)]
struct ClashProxyGroup {
    name: String,
    #[serde(rename = "type")]
    group_type: String,
    proxies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tolerance: Option<u16>,
}

#[derive(Debug, Serialize)]
struct ClashProxy {
    name: String,
    #[serde(rename = "type")]
    proxy_type: String,
    server: String,
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    cipher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
    #[serde(rename = "alterId", skip_serializing_if = "Option::is_none")]
    alter_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    udp: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sni: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    servername: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<String>,
    #[serde(rename = "skip-cert-verify", skip_serializing_if = "Option::is_none")]
    skip_cert_verify: Option<bool>,
    #[serde(rename = "client-fingerprint", skip_serializing_if = "Option::is_none")]
    client_fingerprint: Option<String>,
    #[serde(rename = "ws-opts", skip_serializing_if = "Option::is_none")]
    ws_opts: Option<ClashWsOptions>,
    #[serde(rename = "grpc-opts", skip_serializing_if = "Option::is_none")]
    grpc_opts: Option<ClashGrpcOptions>,
    #[serde(rename = "h2-opts", skip_serializing_if = "Option::is_none")]
    h2_opts: Option<ClashH2Options>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alpn: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obfs: Option<String>,
    #[serde(rename = "obfs-password", skip_serializing_if = "Option::is_none")]
    obfs_password: Option<String>,
    #[serde(
        rename = "congestion-controller",
        skip_serializing_if = "Option::is_none"
    )]
    congestion_control: Option<String>,
    #[serde(rename = "udp-relay-mode", skip_serializing_if = "Option::is_none")]
    udp_relay_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct ClashWsOptions {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(rename = "max-early-data", skip_serializing_if = "Option::is_none")]
    max_early_data: Option<u32>,
    #[serde(
        rename = "early-data-header-name",
        skip_serializing_if = "Option::is_none"
    )]
    early_data_header_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ClashGrpcOptions {
    #[serde(rename = "grpc-service-name")]
    grpc_service_name: String,
}

#[derive(Debug, Serialize)]
struct ClashH2Options {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct SingboxConfig {
    outbounds: Vec<SingboxOutbound>,
}

#[derive(Debug, Serialize)]
struct SingboxOutbound {
    #[serde(rename = "type")]
    outbound_type: String,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    outbounds: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tolerance: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(rename = "server_port", skip_serializing_if = "Option::is_none")]
    server_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    security: Option<String>,
    #[serde(rename = "alter_id", skip_serializing_if = "Option::is_none")]
    alter_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<SingboxTls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<SingboxTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obfs: Option<SingboxObfs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    congestion_control: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    udp_relay_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct SingboxTls {
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    insecure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alpn: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct SingboxTransport {
    #[serde(rename = "type")]
    transport_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_early_data: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    early_data_header_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct SingboxObfs {
    #[serde(rename = "type")]
    obfs_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
    use serde_json::{Value, json};

    use super::{ClashTransformer, SingboxTransformer, Transformer};

    #[test]
    fn snapshot_ss_proxy_yaml() {
        assert_snapshot(
            build_node(
                "SS-HK",
                ProxyProtocol::Ss,
                ProxyTransport::Tcp,
                Some("hk"),
                vec![
                    ("cipher", Value::String("aes-128-gcm".to_string())),
                    ("password", Value::String("p@ss".to_string())),
                ],
            ),
            include_str!("fixtures/clash_ss.yaml"),
        );
    }

    #[test]
    fn snapshot_vmess_proxy_yaml() {
        assert_snapshot(
            build_node(
                "VMESS-SG",
                ProxyProtocol::Vmess,
                ProxyTransport::Ws,
                Some("sg"),
                vec![
                    (
                        "uuid",
                        Value::String("11111111-1111-1111-1111-111111111111".to_string()),
                    ),
                    ("path", Value::String("/ws".to_string())),
                    ("host", Value::String("edge.example.com".to_string())),
                ],
            ),
            include_str!("fixtures/clash_vmess.yaml"),
        );
    }

    #[test]
    fn snapshot_vless_proxy_yaml() {
        assert_snapshot(
            build_node(
                "VLESS-US",
                ProxyProtocol::Vless,
                ProxyTransport::Grpc,
                Some("us"),
                vec![
                    (
                        "uuid",
                        Value::String("22222222-2222-2222-2222-222222222222".to_string()),
                    ),
                    ("service_name", Value::String("vless-grpc".to_string())),
                    ("flow", Value::String("xtls-rprx-vision".to_string())),
                ],
            ),
            include_str!("fixtures/clash_vless.yaml"),
        );
    }

    #[test]
    fn snapshot_trojan_proxy_yaml() {
        assert_snapshot(
            build_node(
                "TROJAN-JP",
                ProxyProtocol::Trojan,
                ProxyTransport::Tcp,
                Some("jp"),
                vec![("password", Value::String("trojan-pass".to_string()))],
            ),
            include_str!("fixtures/clash_trojan.yaml"),
        );
    }

    #[test]
    fn snapshot_hysteria2_proxy_yaml() {
        assert_snapshot(
            build_node(
                "HY2-HK",
                ProxyProtocol::Hysteria2,
                ProxyTransport::Quic,
                Some("hk"),
                vec![
                    ("password", Value::String("hy2-pass".to_string())),
                    ("obfs", Value::String("salamander".to_string())),
                    ("obfs_password", Value::String("hy2-obfs".to_string())),
                    ("alpn", json!(["h3"])),
                ],
            ),
            include_str!("fixtures/clash_hysteria2.yaml"),
        );
    }

    #[test]
    fn snapshot_tuic_proxy_yaml() {
        assert_snapshot(
            build_node(
                "TUIC-SG",
                ProxyProtocol::Tuic,
                ProxyTransport::Quic,
                Some("sg"),
                vec![
                    (
                        "uuid",
                        Value::String("33333333-3333-3333-3333-333333333333".to_string()),
                    ),
                    ("password", Value::String("tuic-pass".to_string())),
                    ("congestion_control", Value::String("bbr".to_string())),
                    ("udp_relay_mode", Value::String("native".to_string())),
                    ("alpn", json!(["h3", "h3-29"])),
                ],
            ),
            include_str!("fixtures/clash_tuic.yaml"),
        );
    }

    #[test]
    fn snapshot_ss_outbound_json() {
        assert_json_snapshot(
            build_node(
                "SS-HK",
                ProxyProtocol::Ss,
                ProxyTransport::Tcp,
                Some("hk"),
                vec![
                    ("cipher", Value::String("aes-128-gcm".to_string())),
                    ("password", Value::String("p@ss".to_string())),
                ],
            ),
            include_str!("fixtures/singbox_ss.json"),
        );
    }

    #[test]
    fn snapshot_vmess_outbound_json() {
        assert_json_snapshot(
            build_node(
                "VMESS-SG",
                ProxyProtocol::Vmess,
                ProxyTransport::Ws,
                Some("sg"),
                vec![
                    (
                        "uuid",
                        Value::String("11111111-1111-1111-1111-111111111111".to_string()),
                    ),
                    ("path", Value::String("/ws".to_string())),
                    ("host", Value::String("edge.example.com".to_string())),
                ],
            ),
            include_str!("fixtures/singbox_vmess.json"),
        );
    }

    #[test]
    fn snapshot_vless_outbound_json() {
        assert_json_snapshot(
            build_node(
                "VLESS-US",
                ProxyProtocol::Vless,
                ProxyTransport::Grpc,
                Some("us"),
                vec![
                    (
                        "uuid",
                        Value::String("22222222-2222-2222-2222-222222222222".to_string()),
                    ),
                    ("service_name", Value::String("vless-grpc".to_string())),
                    ("flow", Value::String("xtls-rprx-vision".to_string())),
                ],
            ),
            include_str!("fixtures/singbox_vless.json"),
        );
    }

    #[test]
    fn snapshot_trojan_outbound_json() {
        assert_json_snapshot(
            build_node(
                "TROJAN-JP",
                ProxyProtocol::Trojan,
                ProxyTransport::Tcp,
                Some("jp"),
                vec![("password", Value::String("trojan-pass".to_string()))],
            ),
            include_str!("fixtures/singbox_trojan.json"),
        );
    }

    #[test]
    fn snapshot_hysteria2_outbound_json() {
        assert_json_snapshot(
            build_node(
                "HY2-HK",
                ProxyProtocol::Hysteria2,
                ProxyTransport::Quic,
                Some("hk"),
                vec![
                    ("password", Value::String("hy2-pass".to_string())),
                    ("obfs", Value::String("salamander".to_string())),
                    ("obfs_password", Value::String("hy2-obfs".to_string())),
                    ("alpn", json!(["h3"])),
                ],
            ),
            include_str!("fixtures/singbox_hysteria2.json"),
        );
    }

    #[test]
    fn snapshot_tuic_outbound_json() {
        assert_json_snapshot(
            build_node(
                "TUIC-SG",
                ProxyProtocol::Tuic,
                ProxyTransport::Quic,
                Some("sg"),
                vec![
                    (
                        "uuid",
                        Value::String("33333333-3333-3333-3333-333333333333".to_string()),
                    ),
                    ("password", Value::String("tuic-pass".to_string())),
                    ("congestion_control", Value::String("bbr".to_string())),
                    ("udp_relay_mode", Value::String("native".to_string())),
                    ("alpn", json!(["h3", "h3-29"])),
                ],
            ),
            include_str!("fixtures/singbox_tuic.json"),
        );
    }

    fn assert_snapshot(node: ProxyNode, expected_snapshot: &str) {
        let transformer = ClashTransformer::default();
        let yaml = transformer
            .transform(&[node], &test_profile())
            .expect("转换 YAML 失败");
        assert_eq!(normalize_yaml(&yaml), normalize_yaml(expected_snapshot));
    }

    fn normalize_yaml(yaml: &str) -> String {
        yaml.replace("\r\n", "\n").trim().to_string()
    }

    fn assert_json_snapshot(node: ProxyNode, expected_snapshot: &str) {
        let transformer = SingboxTransformer::default();
        let json = transformer
            .transform(&[node], &test_profile())
            .expect("转换 JSON 失败");
        assert_eq!(normalize_json(&json), normalize_json(expected_snapshot));
    }

    fn normalize_json(payload: &str) -> String {
        let value: Value = serde_json::from_str(payload).expect("解析 JSON 快照失败");
        serde_json::to_string_pretty(&value).expect("序列化 JSON 快照失败")
    }

    fn test_profile() -> app_common::Profile {
        app_common::Profile {
            id: "profile-1".to_string(),
            name: "Default".to_string(),
            description: Some("test profile".to_string()),
            created_at: "2026-04-03T00:00:00Z".to_string(),
            updated_at: "2026-04-03T00:00:00Z".to_string(),
        }
    }

    fn build_node(
        name: &str,
        protocol: ProxyProtocol,
        transport: ProxyTransport,
        region: Option<&str>,
        extra_entries: Vec<(&str, Value)>,
    ) -> ProxyNode {
        let mut extra = BTreeMap::<String, Value>::new();
        for (key, value) in extra_entries {
            extra.insert(key.to_string(), value);
        }

        ProxyNode {
            id: format!("node-{name}"),
            name: name.to_string(),
            protocol,
            server: format!("{}.example.com", name.to_ascii_lowercase()),
            port: 443,
            transport,
            tls: TlsConfig {
                enabled: true,
                server_name: Some("tls.example.com".to_string()),
            },
            extra,
            source_id: "source-a".to_string(),
            tags: Vec::new(),
            region: region.map(ToString::to_string),
            updated_at: "2026-04-03T00:00:00Z".to_string(),
        }
    }
}
