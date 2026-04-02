use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use base64::Engine as Base64Engine;
use base64::engine::general_purpose::{
    STANDARD as BASE64_STANDARD, STANDARD_NO_PAD as BASE64_STANDARD_NO_PAD,
    URL_SAFE as BASE64_URL_SAFE, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
};
use reqwest::Url;
use serde_json::Value;

use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult};

pub trait SubscriptionParser {
    fn parse(&self, source_id: &str, payload: &str) -> CoreResult<Vec<ProxyNode>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UriListParser;

impl SubscriptionParser for UriListParser {
    fn parse(&self, source_id: &str, payload: &str) -> CoreResult<Vec<ProxyNode>> {
        let normalized = normalize_subscription_payload(payload);
        let updated_at = now_rfc3339()?;
        let mut nodes = Vec::new();

        for (line_number, raw_line) in normalized.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            match parse_proxy_uri_line(line, source_id, &updated_at) {
                Ok(node) => nodes.push(node),
                Err(error) => {
                    eprintln!(
                        "WARN: 解析订阅行失败（source_id={}, line={}）：{}",
                        source_id,
                        line_number + 1,
                        error
                    );
                }
            }
        }

        Ok(nodes)
    }
}

pub(crate) fn normalize_subscription_payload(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if looks_like_uri_list(trimmed) {
        return trimmed.to_string();
    }

    let compact_base64 = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if let Some(decoded) = try_decode_base64_text(&compact_base64) {
        let decoded_trimmed = decoded.trim();
        if looks_like_uri_list(decoded_trimmed) {
            return decoded_trimmed.to_string();
        }
    }

    trimmed.to_string()
}

pub(crate) fn looks_like_uri_list(payload: &str) -> bool {
    payload.contains("ss://")
        || payload.contains("vmess://")
        || payload.contains("vless://")
        || payload.contains("trojan://")
}

pub(crate) fn parse_proxy_uri_line(
    line: &str,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<ProxyNode> {
    if line.starts_with("ss://") {
        return parse_ss_uri(line, source_id, updated_at);
    }
    if line.starts_with("vmess://") {
        return parse_vmess_uri(line, source_id, updated_at);
    }
    if line.starts_with("vless://") {
        return parse_vless_uri(line, source_id, updated_at);
    }
    if line.starts_with("trojan://") {
        return parse_trojan_uri(line, source_id, updated_at);
    }

    Err(CoreError::SubscriptionParse(format!(
        "不支持的 URI 协议：{line}"
    )))
}

pub(crate) fn parse_ss_uri(line: &str, source_id: &str, updated_at: &str) -> CoreResult<ProxyNode> {
    let raw = &line["ss://".len()..];
    let (without_fragment, name) = split_fragment(raw);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);

    let (credential_part, host_part) = if let Some((cred, host)) = without_query.rsplit_once('@') {
        (cred.to_string(), host.to_string())
    } else {
        let decoded = try_decode_base64_text(without_query)
            .ok_or_else(|| CoreError::SubscriptionParse("ss URI 缺少 @server:port".to_string()))?;
        let (cred, host) = decoded
            .rsplit_once('@')
            .ok_or_else(|| CoreError::SubscriptionParse("ss URI 凭证无法解析".to_string()))?;
        (cred.to_string(), host.to_string())
    };

    let credential_decoded =
        try_decode_base64_text(&credential_part).unwrap_or_else(|| credential_part.clone());
    let (cipher, password) = credential_decoded.split_once(':').ok_or_else(|| {
        CoreError::SubscriptionParse("ss URI 凭证必须为 method:password".to_string())
    })?;
    let (server, port) = parse_host_port(&host_part)?;

    let mut extra = BTreeMap::new();
    extra.insert("cipher".to_string(), Value::String(cipher.to_string()));
    extra.insert("password".to_string(), Value::String(password.to_string()));

    Ok(build_proxy_node(
        source_id,
        name.unwrap_or_else(|| format!("ss-{server}:{port}")),
        ProxyProtocol::Ss,
        server,
        port,
        ProxyTransport::Tcp,
        TlsConfig {
            enabled: false,
            server_name: None,
        },
        extra,
        updated_at,
    ))
}

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
    let name = payload
        .get("ps")
        .and_then(Value::as_str)
        .unwrap_or("vmess")
        .to_string();
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

pub(crate) fn parse_vless_uri(
    line: &str,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<ProxyNode> {
    let url = Url::parse(line)
        .map_err(|error| CoreError::SubscriptionParse(format!("vless URI 非法：{error}")))?;
    let server = url
        .host_str()
        .ok_or_else(|| CoreError::SubscriptionParse("vless URI 缺少 host".to_string()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CoreError::SubscriptionParse("vless URI 缺少端口".to_string()))?;
    let name = url
        .fragment()
        .filter(|value| !value.is_empty())
        .unwrap_or("vless")
        .to_string();
    let transport = map_transport(url.query_pairs().find_map(|(k, v)| {
        if k == "type" {
            Some(v.to_string())
        } else {
            None
        }
    }));
    let security = url.query_pairs().find_map(|(k, v)| {
        if k == "security" {
            Some(v.to_string())
        } else {
            None
        }
    });
    let sni = url.query_pairs().find_map(|(k, v)| {
        if k == "sni" {
            Some(v.to_string())
        } else {
            None
        }
    });

    let mut extra = BTreeMap::new();
    if !url.username().is_empty() {
        extra.insert(
            "uuid".to_string(),
            Value::String(url.username().to_string()),
        );
    }

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Vless,
        server,
        port,
        transport,
        TlsConfig {
            enabled: matches!(security.as_deref(), Some("tls" | "reality" | "xtls")),
            server_name: sni,
        },
        extra,
        updated_at,
    ))
}

pub(crate) fn parse_trojan_uri(
    line: &str,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<ProxyNode> {
    let url = Url::parse(line)
        .map_err(|error| CoreError::SubscriptionParse(format!("trojan URI 非法：{error}")))?;
    let server = url
        .host_str()
        .ok_or_else(|| CoreError::SubscriptionParse("trojan URI 缺少 host".to_string()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CoreError::SubscriptionParse("trojan URI 缺少端口".to_string()))?;
    let name = url
        .fragment()
        .filter(|value| !value.is_empty())
        .unwrap_or("trojan")
        .to_string();

    let mut extra = BTreeMap::new();
    if !url.username().is_empty() {
        extra.insert(
            "password".to_string(),
            Value::String(url.username().to_string()),
        );
    }
    let sni = url.query_pairs().find_map(|(k, v)| {
        if k == "sni" {
            Some(v.to_string())
        } else {
            None
        }
    });

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Trojan,
        server,
        port,
        ProxyTransport::Tcp,
        TlsConfig {
            enabled: true,
            server_name: sni,
        },
        extra,
        updated_at,
    ))
}

pub(crate) fn split_fragment(raw: &str) -> (&str, Option<String>) {
    if let Some((value, fragment)) = raw.split_once('#') {
        (value, Some(fragment.to_string()))
    } else {
        (raw, None)
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

pub(crate) fn try_decode_base64_text(raw: &str) -> Option<String> {
    for engine in [
        &BASE64_STANDARD,
        &BASE64_STANDARD_NO_PAD,
        &BASE64_URL_SAFE,
        &BASE64_URL_SAFE_NO_PAD,
    ] {
        if let Ok(bytes) = engine.decode(raw) {
            if let Ok(text) = String::from_utf8(bytes) {
                return Some(text);
            }
        }
    }
    None
}
