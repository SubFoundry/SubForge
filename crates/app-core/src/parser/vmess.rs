use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use serde_json::Value;

use crate::CoreError;
use crate::CoreResult;

use super::{build_proxy_node, decode_percent_encoded, try_decode_base64_text};

pub(crate) fn parse_vmess_uri(
    line: &str,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<ProxyNode> {
    let raw = line["vmess://".len()..].trim();
    let decoded = try_decode_base64_text(raw)
        .ok_or_else(|| CoreError::SubscriptionParse("vmess URI Base64 解码失败".to_string()))?;
    let payload = serde_json::from_str::<Value>(&decoded)
        .map_err(|error| CoreError::SubscriptionParse(format!("vmess JSON 非法：{error}")))?;

    let server = payload
        .get("add")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::SubscriptionParse("vmess 缺少 add".to_string()))?
        .to_string();
    let port = payload
        .get("port")
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|raw| u16::try_from(raw).ok())
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<u16>().ok()))
        })
        .ok_or_else(|| CoreError::SubscriptionParse("vmess 缺少有效 port".to_string()))?;
    let name = decode_percent_encoded(payload.get("ps").and_then(Value::as_str).unwrap_or("vmess"));
    let transport = match payload.get("net").and_then(Value::as_str) {
        Some("ws") => ProxyTransport::Ws,
        Some("grpc") => ProxyTransport::Grpc,
        Some("h2") => ProxyTransport::H2,
        Some("quic") => ProxyTransport::Quic,
        _ => ProxyTransport::Tcp,
    };

    let tls_enabled = matches!(
        payload.get("tls").and_then(value_to_string).as_deref(),
        Some("tls" | "reality")
    );
    let server_name = payload
        .get("sni")
        .and_then(value_to_string)
        .or_else(|| first_host(payload.get("host"), transport.clone()));

    let mut extra = BTreeMap::new();
    insert_optional_string(
        &mut extra,
        "uuid",
        payload.get("id").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "path",
        payload.get("path").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "flow",
        payload.get("flow").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "service_name",
        payload
            .get("serviceName")
            .and_then(value_to_string)
            .or_else(|| payload.get("service_name").and_then(value_to_string)),
    );
    insert_optional_string(
        &mut extra,
        "grpc_service_name",
        payload.get("grpc_service_name").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "client_fingerprint",
        payload.get("fp").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "security",
        payload.get("security").and_then(value_to_string),
    );
    insert_optional_string(
        &mut extra,
        "cipher",
        payload
            .get("scy")
            .and_then(value_to_string)
            .or_else(|| payload.get("cipher").and_then(value_to_string)),
    );
    insert_optional_u64(
        &mut extra,
        "alter_id",
        payload.get("aid").and_then(value_to_u64),
    );
    insert_optional_bool(
        &mut extra,
        "skip_cert_verify",
        payload
            .get("allowInsecure")
            .and_then(value_to_bool)
            .or_else(|| payload.get("allow_insecure").and_then(value_to_bool)),
    );
    insert_optional_alpn(
        &mut extra,
        payload
            .get("alpn")
            .and_then(value_to_string)
            .map(|value| split_csv_values(&value))
            .or_else(|| payload.get("alpn").and_then(value_to_string_list)),
    );
    insert_transport_host(&mut extra, payload.get("host"), transport.clone());

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Vmess,
        server,
        port,
        transport,
        TlsConfig {
            enabled: tls_enabled,
            server_name,
        },
        extra,
        updated_at,
    ))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(raw) => Some(if *raw { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn value_to_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn value_to_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(raw) => Some(*raw),
        Value::Number(number) => number.as_u64().map(|value| value != 0),
        Value::String(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_to_string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(|item| match item {
                    Value::String(raw) => {
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        }
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        _ => None,
    }
}

fn insert_optional_string(extra: &mut BTreeMap<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extra.insert(key.to_string(), Value::String(value));
    }
}

fn insert_optional_u64(extra: &mut BTreeMap<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        extra.insert(key.to_string(), Value::Number(value.into()));
    }
}

fn insert_optional_bool(extra: &mut BTreeMap<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        extra.insert(key.to_string(), Value::Bool(value));
    }
}

fn insert_optional_alpn(extra: &mut BTreeMap<String, Value>, values: Option<Vec<String>>) {
    let Some(values) = values else {
        return;
    };
    if values.is_empty() {
        return;
    }
    extra.insert(
        "alpn".to_string(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
}

fn insert_transport_host(
    extra: &mut BTreeMap<String, Value>,
    host_value: Option<&Value>,
    transport: ProxyTransport,
) {
    let Some(host_value) = host_value else {
        return;
    };
    let hosts = hosts_from_value(host_value);
    if hosts.is_empty() {
        return;
    }
    if matches!(transport, ProxyTransport::H2) {
        extra.insert(
            "host".to_string(),
            Value::Array(hosts.into_iter().map(Value::String).collect()),
        );
    } else if let Some(first) = hosts.into_iter().next() {
        extra.insert("host".to_string(), Value::String(first));
    }
}

fn hosts_from_value(value: &Value) -> Vec<String> {
    match value {
        Value::String(raw) => split_csv_values(raw),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(raw) => Some(raw.as_str()),
                _ => None,
            })
            .flat_map(split_csv_values)
            .collect(),
        _ => Vec::new(),
    }
}

fn first_host(host_value: Option<&Value>, transport: ProxyTransport) -> Option<String> {
    if matches!(transport, ProxyTransport::H2) {
        return None;
    }
    host_value
        .and_then(|value| hosts_from_value(value).into_iter().next())
        .filter(|value| !value.is_empty())
}

fn split_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}
