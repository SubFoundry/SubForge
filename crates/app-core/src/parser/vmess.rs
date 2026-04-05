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
        payload.get("tls").and_then(Value::as_str),
        Some("tls" | "reality")
    );
    let server_name = payload
        .get("sni")
        .and_then(Value::as_str)
        .or_else(|| payload.get("host").and_then(Value::as_str))
        .map(ToString::to_string);

    let mut extra = BTreeMap::new();
    if let Some(uuid) = payload.get("id").and_then(Value::as_str) {
        extra.insert("uuid".to_string(), Value::String(uuid.to_string()));
    }
    if let Some(path) = payload.get("path").and_then(Value::as_str) {
        extra.insert("path".to_string(), Value::String(path.to_string()));
    }

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
