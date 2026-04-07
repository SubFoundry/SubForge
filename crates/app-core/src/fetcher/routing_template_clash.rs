use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};

use crate::parser::{build_proxy_node, map_transport};
use crate::utils::{now_rfc3339, safe_stderr_line};
use crate::{CoreError, CoreResult};

use super::utils::yaml_map_get;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedClashPayload {
    pub(super) nodes: Vec<ProxyNode>,
    pub(super) base_config_yaml: Option<String>,
}

pub(super) fn parse_clash_payload(
    source_id: &str,
    payload: &str,
) -> CoreResult<Option<ParsedClashPayload>> {
    let root = serde_yaml::from_str::<YamlValue>(payload).ok();
    let Some(root) = root.and_then(|value| value.as_mapping().cloned()) else {
        return Ok(None);
    };
    if !looks_like_clash_payload(&root) {
        return Ok(None);
    }

    let updated_at = now_rfc3339()?;
    let nodes = parse_clash_nodes(&root, source_id, &updated_at);
    let base_config_yaml = build_base_config_yaml(&root)?;
    Ok(Some(ParsedClashPayload {
        nodes,
        base_config_yaml,
    }))
}

fn looks_like_clash_payload(root: &YamlMapping) -> bool {
    ["proxies", "proxy-groups", "rules"]
        .iter()
        .any(|key| yaml_map_get(root, key).is_some())
}

fn build_base_config_yaml(root: &YamlMapping) -> CoreResult<Option<String>> {
    let mut base = YamlMapping::new();
    for (key, value) in root {
        let key_name = key.as_str().map(str::trim);
        if matches!(key_name, Some("proxies" | "proxy-groups" | "rules")) {
            continue;
        }
        base.insert(key.clone(), value.clone());
    }

    if base.is_empty() {
        return Ok(None);
    }

    serde_yaml::to_string(&YamlValue::Mapping(base))
        .map(Some)
        .map_err(|error| {
            CoreError::ConfigInvalid(format!("序列化 Clash 母版基础配置失败：{error}"))
        })
}

fn parse_clash_nodes(root: &YamlMapping, source_id: &str, updated_at: &str) -> Vec<ProxyNode> {
    let Some(items) = yaml_map_get(root, "proxies").and_then(YamlValue::as_sequence) else {
        return Vec::new();
    };

    let mut nodes = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match parse_clash_proxy(item, source_id, updated_at) {
            Ok(node) => nodes.push(node),
            Err(error) => safe_stderr_line(&format!(
                "WARN: 解析 Clash YAML 节点失败（source_id={}, index={}）：{}",
                source_id,
                index + 1,
                error
            )),
        }
    }

    nodes
}

fn parse_clash_proxy(item: &YamlValue, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let proxy = serde_yaml::from_value::<RawClashProxy>(item.clone())
        .map_err(|error| CoreError::SubscriptionParse(format!("Clash 节点结构非法：{error}")))?;
    let proxy_type = proxy.proxy_type.trim().to_ascii_lowercase();

    let protocol = match proxy_type.as_str() {
        "ss" => ProxyProtocol::Ss,
        "vmess" => ProxyProtocol::Vmess,
        "vless" => ProxyProtocol::Vless,
        "trojan" => ProxyProtocol::Trojan,
        "hysteria2" => ProxyProtocol::Hysteria2,
        "tuic" => ProxyProtocol::Tuic,
        _ => {
            return Err(CoreError::SubscriptionParse(format!(
                "不支持的 Clash 节点类型：{}",
                proxy.proxy_type
            )));
        }
    };

    let name = require_non_empty(proxy.name, "name")?;
    let server = require_non_empty(proxy.server, "server")?;
    let transport = resolve_transport(&protocol, proxy.network.as_deref());
    let tls = TlsConfig {
        enabled: proxy.tls.unwrap_or(matches!(
            protocol,
            ProxyProtocol::Vmess
                | ProxyProtocol::Vless
                | ProxyProtocol::Trojan
                | ProxyProtocol::Hysteria2
                | ProxyProtocol::Tuic
        )),
        server_name: proxy.servername.or(proxy.sni.clone()),
    };

    let mut extra = BTreeMap::<String, JsonValue>::new();
    insert_optional_string(&mut extra, "cipher", proxy.cipher);
    insert_optional_string(&mut extra, "password", proxy.password);
    insert_optional_string(&mut extra, "uuid", proxy.uuid);
    insert_optional_u64(&mut extra, "alter_id", proxy.alter_id);
    insert_optional_string(&mut extra, "flow", proxy.flow);
    insert_optional_bool(&mut extra, "skip_cert_verify", proxy.skip_cert_verify);
    insert_optional_string(&mut extra, "client_fingerprint", proxy.client_fingerprint);
    insert_optional_string_list(&mut extra, "alpn", proxy.alpn);
    insert_optional_string(&mut extra, "obfs", proxy.obfs);
    insert_optional_string(&mut extra, "obfs_password", proxy.obfs_password);
    insert_optional_string(
        &mut extra,
        "congestion_control",
        proxy.congestion_controller,
    );
    insert_optional_string(&mut extra, "udp_relay_mode", proxy.udp_relay_mode);

    if let Some(ws_opts) = proxy.ws_opts {
        insert_optional_string(&mut extra, "path", ws_opts.path);
        if let Some(headers) = ws_opts.headers {
            let host = headers
                .get("Host")
                .or_else(|| headers.get("host"))
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            insert_optional_string(&mut extra, "host", host);
        }
    }
    if let Some(grpc_opts) = proxy.grpc_opts {
        insert_optional_string(&mut extra, "grpc_service_name", grpc_opts.grpc_service_name);
    }
    if let Some(h2_opts) = proxy.h2_opts {
        insert_optional_string(&mut extra, "path", h2_opts.path);
        insert_optional_string_list(&mut extra, "host", h2_opts.host);
    }

    validate_required_fields(&protocol, &extra, &name)?;

    Ok(build_proxy_node(
        source_id, name, protocol, server, proxy.port, transport, tls, extra, updated_at,
    ))
}

fn resolve_transport(protocol: &ProxyProtocol, network: Option<&str>) -> ProxyTransport {
    match protocol {
        ProxyProtocol::Hysteria2 | ProxyProtocol::Tuic => ProxyTransport::Quic,
        _ => map_transport(
            network
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        ),
    }
}

fn validate_required_fields(
    protocol: &ProxyProtocol,
    extra: &BTreeMap<String, JsonValue>,
    node_name: &str,
) -> CoreResult<()> {
    let required = match protocol {
        ProxyProtocol::Ss => &["cipher", "password"][..],
        ProxyProtocol::Vmess => &["uuid"][..],
        ProxyProtocol::Vless => &["uuid"][..],
        ProxyProtocol::Trojan => &["password"][..],
        ProxyProtocol::Hysteria2 => &["password"][..],
        ProxyProtocol::Tuic => &["uuid", "password"][..],
    };

    for key in required {
        if !extra.contains_key(*key) {
            return Err(CoreError::SubscriptionParse(format!(
                "Clash 节点缺少必要字段：{} ({})",
                key, node_name
            )));
        }
    }

    Ok(())
}

fn require_non_empty(value: String, field: &str) -> CoreResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CoreError::SubscriptionParse(format!(
            "Clash 节点缺少必要字段：{field}"
        )))
    } else {
        Ok(trimmed.to_string())
    }
}

fn insert_optional_string(
    extra: &mut BTreeMap<String, JsonValue>,
    key: &str,
    value: Option<String>,
) {
    let Some(value) = value.map(|item| item.trim().to_string()) else {
        return;
    };
    if value.is_empty() {
        return;
    }
    extra.insert(key.to_string(), JsonValue::String(value));
}

fn insert_optional_u64(extra: &mut BTreeMap<String, JsonValue>, key: &str, value: Option<u64>) {
    let Some(value) = value else {
        return;
    };
    extra.insert(key.to_string(), JsonValue::Number(value.into()));
}

fn insert_optional_bool(extra: &mut BTreeMap<String, JsonValue>, key: &str, value: Option<bool>) {
    let Some(value) = value else {
        return;
    };
    extra.insert(key.to_string(), JsonValue::Bool(value));
}

fn insert_optional_string_list(
    extra: &mut BTreeMap<String, JsonValue>,
    key: &str,
    value: Option<Vec<String>>,
) {
    let Some(values) = value else {
        return;
    };
    let values = values
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .map(JsonValue::String)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    extra.insert(key.to_string(), JsonValue::Array(values));
}

#[derive(Debug, Deserialize)]
struct RawClashProxy {
    name: String,
    #[serde(rename = "type")]
    proxy_type: String,
    server: String,
    port: u16,
    #[serde(default)]
    cipher: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(rename = "alterId", default)]
    alter_id: Option<u64>,
    #[serde(default)]
    tls: Option<bool>,
    #[serde(default)]
    sni: Option<String>,
    #[serde(default)]
    servername: Option<String>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    flow: Option<String>,
    #[serde(rename = "skip-cert-verify", default)]
    skip_cert_verify: Option<bool>,
    #[serde(rename = "client-fingerprint", default)]
    client_fingerprint: Option<String>,
    #[serde(rename = "ws-opts", default)]
    ws_opts: Option<RawClashWsOptions>,
    #[serde(rename = "grpc-opts", default)]
    grpc_opts: Option<RawClashGrpcOptions>,
    #[serde(rename = "h2-opts", default)]
    h2_opts: Option<RawClashH2Options>,
    #[serde(default)]
    alpn: Option<Vec<String>>,
    #[serde(default)]
    obfs: Option<String>,
    #[serde(rename = "obfs-password", default)]
    obfs_password: Option<String>,
    #[serde(rename = "congestion-controller", default)]
    congestion_controller: Option<String>,
    #[serde(rename = "udp-relay-mode", default)]
    udp_relay_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawClashWsOptions {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct RawClashGrpcOptions {
    #[serde(rename = "grpc-service-name", default)]
    grpc_service_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawClashH2Options {
    #[serde(default)]
    host: Option<Vec<String>>,
    #[serde(default)]
    path: Option<String>,
}
