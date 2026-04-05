use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use serde_json::Value;

use crate::{CoreError, CoreResult};

pub(crate) fn split_fragment(raw: &str) -> (&str, Option<String>) {
    if let Some((value, fragment)) = raw.split_once('#') {
        (value, Some(decode_percent_encoded(fragment)))
    } else {
        (raw, None)
    }
}

pub(crate) fn decode_percent_encoded(raw: &str) -> String {
    if !raw.as_bytes().contains(&b'%') {
        return raw.to_string();
    }

    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = bytes[index + 1];
            let lo = bytes[index + 2];
            if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit() {
                let value = (hex_value(hi) << 4) | hex_value(lo);
                decoded.push(value);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    match String::from_utf8(decoded) {
        Ok(value) => value,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

pub(crate) fn parse_host_port(raw: &str) -> CoreResult<(String, u16)> {
    if let Some(stripped) = raw.strip_prefix('[') {
        let (host, remainder) = stripped
            .split_once(']')
            .ok_or_else(|| CoreError::SubscriptionParse(format!("host 非法：{raw}")))?;
        let port = remainder
            .strip_prefix(':')
            .ok_or_else(|| CoreError::SubscriptionParse(format!("端口缺失：{raw}")))?
            .parse::<u16>()
            .map_err(|error| CoreError::SubscriptionParse(format!("端口非法：{error}")))?;
        return Ok((host.to_string(), port));
    }

    let (host, port) = raw
        .rsplit_once(':')
        .ok_or_else(|| CoreError::SubscriptionParse(format!("host:port 解析失败：{raw}")))?;
    let port = port
        .parse::<u16>()
        .map_err(|error| CoreError::SubscriptionParse(format!("端口非法：{error}")))?;
    Ok((host.to_string(), port))
}

pub(crate) fn map_transport(raw: Option<String>) -> ProxyTransport {
    match raw.as_deref() {
        Some("ws") => ProxyTransport::Ws,
        Some("grpc") => ProxyTransport::Grpc,
        Some("h2") => ProxyTransport::H2,
        Some("quic") => ProxyTransport::Quic,
        _ => ProxyTransport::Tcp,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_proxy_node(
    source_id: &str,
    name: String,
    protocol: ProxyProtocol,
    server: String,
    port: u16,
    transport: ProxyTransport,
    tls: TlsConfig,
    extra: BTreeMap<String, Value>,
    updated_at: &str,
) -> ProxyNode {
    ProxyNode {
        id: build_proxy_node_id(
            source_id,
            &protocol,
            &server,
            port,
            &name,
            extra.get("uuid").or_else(|| extra.get("password")),
        ),
        name,
        protocol,
        server,
        port,
        transport,
        tls,
        extra,
        source_id: source_id.to_string(),
        tags: Vec::new(),
        region: None,
        updated_at: updated_at.to_string(),
    }
}

pub(crate) fn build_proxy_node_id(
    source_id: &str,
    protocol: &ProxyProtocol,
    server: &str,
    port: u16,
    name: &str,
    credential: Option<&Value>,
) -> String {
    let mut hasher = DefaultHasher::new();
    source_id.hash(&mut hasher);
    protocol.hash(&mut hasher);
    server.hash(&mut hasher);
    port.hash(&mut hasher);
    name.hash(&mut hasher);
    if let Some(value) = credential {
        value.to_string().hash(&mut hasher);
    }
    format!("node-{:016x}", hasher.finish())
}
