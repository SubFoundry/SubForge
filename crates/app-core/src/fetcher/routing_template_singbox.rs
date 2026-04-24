use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use serde_json::Value as JsonValue;

use crate::parser::build_proxy_node;
use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedSingboxPayload {
    pub(super) nodes: Vec<ProxyNode>,
}

pub(super) fn parse_singbox_payload(
    source_id: &str,
    payload: &str,
) -> CoreResult<Option<ParsedSingboxPayload>> {
    let root = serde_json::from_str::<JsonValue>(payload).ok();
    let Some(outbounds) = root
        .as_ref()
        .and_then(|value| value.get("outbounds"))
        .and_then(JsonValue::as_array)
    else {
        return Ok(None);
    };

    let updated_at = now_rfc3339()?;
    let mut nodes = Vec::new();
    let mut recognized = false;
    for outbound in outbounds {
        if is_singbox_group_outbound(outbound) {
            recognized = true;
            continue;
        }
        let Some(node) = parse_singbox_node(outbound, source_id, &updated_at)? else {
            continue;
        };
        recognized = true;
        nodes.push(node);
    }

    if !recognized {
        return Ok(None);
    }

    Ok(Some(ParsedSingboxPayload { nodes }))
}

fn is_singbox_group_outbound(outbound: &JsonValue) -> bool {
    outbound
        .get("type")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .is_some_and(|value| matches!(value, "selector" | "urltest"))
}

fn parse_singbox_node(
    outbound: &JsonValue,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<Option<ProxyNode>> {
    let map = outbound
        .as_object()
        .ok_or_else(|| CoreError::SubscriptionParse("sing-box outbound 结构非法".to_string()))?;
    let outbound_type = required_string(map.get("type"), "type")?;
    let protocol = match outbound_type.as_str() {
        "selector" | "urltest" => return Ok(None),
        "shadowsocks" => ProxyProtocol::Ss,
        "vmess" => ProxyProtocol::Vmess,
        "vless" => ProxyProtocol::Vless,
        "trojan" => ProxyProtocol::Trojan,
        "hysteria2" => ProxyProtocol::Hysteria2,
        "tuic" => ProxyProtocol::Tuic,
        "anytls" => ProxyProtocol::AnyTls,
        _ => return Ok(None),
    };

    let name = required_string(map.get("tag"), "tag")?;
    let server = required_string(map.get("server"), "server")?;
    let port = required_u16(map.get("server_port"), "server_port")?;
    let tls = parse_singbox_tls(map.get("tls"));
    let transport = resolve_transport(&protocol, map.get("transport"));

    let mut extra = BTreeMap::<String, JsonValue>::new();
    match protocol {
        ProxyProtocol::Ss => {
            insert_optional_string(&mut extra, "cipher", string_value(map.get("method")));
            insert_optional_string(&mut extra, "password", string_value(map.get("password")));
        }
        ProxyProtocol::Vmess => {
            insert_optional_string(&mut extra, "uuid", string_value(map.get("uuid")));
            insert_optional_string(
                &mut extra,
                "cipher",
                string_value(map.get("security")).or_else(|| Some("auto".to_string())),
            );
            insert_optional_u64(&mut extra, "alter_id", u64_value(map.get("alter_id")));
        }
        ProxyProtocol::Vless => {
            insert_optional_string(&mut extra, "uuid", string_value(map.get("uuid")));
            insert_optional_string(&mut extra, "flow", string_value(map.get("flow")));
        }
        ProxyProtocol::Trojan => {
            insert_optional_string(&mut extra, "password", string_value(map.get("password")));
        }
        ProxyProtocol::Hysteria2 => {
            insert_optional_string(&mut extra, "password", string_value(map.get("password")));
            if let Some(obfs) = map.get("obfs").and_then(JsonValue::as_object) {
                insert_optional_string(&mut extra, "obfs", string_value(obfs.get("type")));
                insert_optional_string(
                    &mut extra,
                    "obfs_password",
                    string_value(obfs.get("password")),
                );
            }
        }
        ProxyProtocol::Tuic => {
            insert_optional_string(&mut extra, "uuid", string_value(map.get("uuid")));
            insert_optional_string(&mut extra, "password", string_value(map.get("password")));
            insert_optional_string(
                &mut extra,
                "congestion_control",
                string_value(map.get("congestion_control")),
            );
            insert_optional_string(
                &mut extra,
                "udp_relay_mode",
                string_value(map.get("udp_relay_mode")),
            );
        }
        ProxyProtocol::AnyTls => {
            insert_optional_string(&mut extra, "password", string_value(map.get("password")));
        }
    }

    if let Some(transport_map) = map.get("transport").and_then(JsonValue::as_object) {
        insert_optional_string(&mut extra, "path", string_value(transport_map.get("path")));
        insert_optional_string(
            &mut extra,
            "service_name",
            string_value(transport_map.get("service_name")),
        );
        insert_optional_u64(
            &mut extra,
            "max_early_data",
            u64_value(transport_map.get("max_early_data")),
        );
        insert_optional_string(
            &mut extra,
            "early_data_header_name",
            string_value(transport_map.get("early_data_header_name")),
        );

        if let Some(headers) = transport_map.get("headers").and_then(JsonValue::as_object) {
            insert_optional_string(&mut extra, "host", string_value(headers.get("Host")));
        }
        if let Some(host) = transport_map.get("host") {
            match resolve_transport(&protocol, map.get("transport")) {
                ProxyTransport::Ws => {
                    insert_optional_string(&mut extra, "host", string_value(Some(host)));
                }
                ProxyTransport::H2 => {
                    insert_optional_string_list(&mut extra, "host", string_list_value(Some(host)));
                }
                _ => {}
            }
        }
    }

    if let Some(tls_map) = map.get("tls").and_then(JsonValue::as_object) {
        insert_optional_bool(
            &mut extra,
            "skip_cert_verify",
            bool_value(tls_map.get("insecure")),
        );
        insert_optional_string_list(&mut extra, "alpn", string_list_value(tls_map.get("alpn")));
    }

    validate_required_fields(&protocol, &extra, &name)?;

    Ok(Some(build_proxy_node(
        source_id, name, protocol, server, port, transport, tls, extra, updated_at,
    )))
}

fn parse_singbox_tls(value: Option<&JsonValue>) -> TlsConfig {
    let Some(map) = value.and_then(JsonValue::as_object) else {
        return TlsConfig {
            enabled: false,
            server_name: None,
        };
    };

    TlsConfig {
        enabled: bool_value(map.get("enabled")).unwrap_or(true),
        server_name: string_value(map.get("server_name")),
    }
}

fn resolve_transport(protocol: &ProxyProtocol, value: Option<&JsonValue>) -> ProxyTransport {
    match protocol {
        ProxyProtocol::Hysteria2 | ProxyProtocol::Tuic => ProxyTransport::Quic,
        _ => match value
            .and_then(JsonValue::as_object)
            .and_then(|map| map.get("type"))
            .and_then(JsonValue::as_str)
            .map(str::trim)
        {
            Some("ws") => ProxyTransport::Ws,
            Some("grpc") => ProxyTransport::Grpc,
            Some("http") => ProxyTransport::H2,
            Some("quic") => ProxyTransport::Quic,
            _ => ProxyTransport::Tcp,
        },
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
        ProxyProtocol::AnyTls => &["password"][..],
    };

    for key in required {
        if !extra.contains_key(*key) {
            return Err(CoreError::SubscriptionParse(format!(
                "sing-box 节点缺少必要字段：{} ({})",
                key, node_name
            )));
        }
    }

    Ok(())
}

fn required_string(value: Option<&JsonValue>, field: &str) -> CoreResult<String> {
    string_value(value)
        .ok_or_else(|| CoreError::SubscriptionParse(format!("sing-box 节点缺少必要字段：{field}")))
}

fn required_u16(value: Option<&JsonValue>, field: &str) -> CoreResult<u16> {
    u64_value(value)
        .and_then(|raw| u16::try_from(raw).ok())
        .ok_or_else(|| CoreError::SubscriptionParse(format!("sing-box 节点缺少必要字段：{field}")))
}

fn string_value(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    match value {
        JsonValue::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(value) => Some(if *value { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn string_list_value(value: Option<&JsonValue>) -> Option<Vec<String>> {
    let value = value?;
    if let Some(single) = string_value(Some(value)) {
        return Some(vec![single]);
    }
    let items = value
        .as_array()?
        .iter()
        .filter_map(|item| string_value(Some(item)))
        .collect::<Vec<_>>();
    if items.is_empty() { None } else { Some(items) }
}

fn u64_value(value: Option<&JsonValue>) -> Option<u64> {
    let value = value?;
    match value {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn bool_value(value: Option<&JsonValue>) -> Option<bool> {
    let value = value?;
    match value {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_u64().map(|raw| raw != 0),
        JsonValue::String(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn insert_optional_string(
    extra: &mut BTreeMap<String, JsonValue>,
    key: &str,
    value: Option<String>,
) {
    let Some(value) = value else {
        return;
    };
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
    extra.insert(
        key.to_string(),
        JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
    );
}
