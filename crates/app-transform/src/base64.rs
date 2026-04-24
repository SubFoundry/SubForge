use app_common::{Profile, ProxyNode, ProxyProtocol, ProxyTransport};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use serde_json::Value;

use crate::shared::{
    clash_network, optional_bool, optional_string, optional_string_list, optional_u32,
    required_string,
};
use crate::{TransformError, TransformResult, Transformer};

#[derive(Debug, Clone, Default)]
pub struct Base64Transformer;

impl Transformer for Base64Transformer {
    fn transform(&self, nodes: &[ProxyNode], _profile: &Profile) -> TransformResult<String> {
        let mut uri_lines = Vec::with_capacity(nodes.len());
        for node in nodes {
            uri_lines.push(build_share_uri(node)?);
        }

        let merged = uri_lines.join("\n");
        Ok(BASE64_STANDARD.encode(merged.as_bytes()))
    }
}

fn build_share_uri(node: &ProxyNode) -> TransformResult<String> {
    match node.protocol {
        ProxyProtocol::Ss => build_ss_uri(node),
        ProxyProtocol::Vmess => build_vmess_uri(node),
        ProxyProtocol::Vless => build_vless_uri(node),
        ProxyProtocol::Trojan => build_trojan_uri(node),
        ProxyProtocol::Hysteria2 => build_hysteria2_uri(node),
        ProxyProtocol::Tuic => build_tuic_uri(node),
        ProxyProtocol::AnyTls => build_anytls_uri(node),
    }
}

fn build_ss_uri(node: &ProxyNode) -> TransformResult<String> {
    let cipher = required_string(node, "cipher")?;
    let password = required_string(node, "password")?;
    let credential = format!("{cipher}:{password}");
    let encoded_credential = URL_SAFE_NO_PAD.encode(credential.as_bytes());

    Ok(format!(
        "ss://{encoded_credential}@{}:{}#{}",
        format_host(&node.server),
        node.port,
        percent_encode_fragment(&node.name),
    ))
}

fn build_vmess_uri(node: &ProxyNode) -> TransformResult<String> {
    let mut payload = serde_json::Map::<String, Value>::new();
    payload.insert("v".to_string(), Value::String("2".to_string()));
    payload.insert("ps".to_string(), Value::String(node.name.clone()));
    payload.insert("add".to_string(), Value::String(node.server.clone()));
    payload.insert("port".to_string(), Value::String(node.port.to_string()));
    payload.insert(
        "id".to_string(),
        Value::String(required_string(node, "uuid")?),
    );
    payload.insert(
        "aid".to_string(),
        Value::String(optional_u32(node, "alter_id").unwrap_or(0).to_string()),
    );
    payload.insert(
        "scy".to_string(),
        Value::String(
            optional_string(node, "security")
                .or_else(|| optional_string(node, "cipher"))
                .unwrap_or_else(|| "auto".to_string()),
        ),
    );
    payload.insert(
        "net".to_string(),
        Value::String(clash_network(&node.transport).to_string()),
    );
    payload.insert("type".to_string(), Value::String("none".to_string()));
    if let Some(host) = optional_string(node, "host") {
        payload.insert("host".to_string(), Value::String(host));
    }
    if let Some(path) = optional_string(node, "path") {
        payload.insert("path".to_string(), Value::String(path));
    }
    if let Some(service_name) =
        optional_string(node, "grpc_service_name").or_else(|| optional_string(node, "service_name"))
    {
        payload.insert("serviceName".to_string(), Value::String(service_name));
    }
    if node.tls.enabled {
        payload.insert("tls".to_string(), Value::String("tls".to_string()));
    }
    if let Some(sni) = node_server_name(node) {
        payload.insert("sni".to_string(), Value::String(sni));
    }
    if let Some(alpn) = optional_string_list(node, "alpn") {
        payload.insert("alpn".to_string(), Value::String(alpn.join(",")));
    }
    if let Some(fingerprint) = optional_string(node, "client_fingerprint") {
        payload.insert("fp".to_string(), Value::String(fingerprint));
    }

    let encoded = BASE64_STANDARD.encode(serde_json::to_string(&payload)?.as_bytes());
    Ok(format!("vmess://{encoded}"))
}

fn build_vless_uri(node: &ProxyNode) -> TransformResult<String> {
    let uuid = required_string(node, "uuid")?;
    let mut params = Vec::<(String, String)>::new();
    push_query_param(&mut params, "encryption", Some("none".to_string()));
    append_transport_params(node, &mut params);
    if node.tls.enabled {
        push_query_param(&mut params, "security", Some("tls".to_string()));
    } else if let Some(security) = optional_string(node, "security") {
        push_query_param(&mut params, "security", Some(security));
    }
    push_query_param(&mut params, "sni", node_server_name(node));
    push_query_param(&mut params, "flow", optional_string(node, "flow"));
    if optional_bool(node, "skip_cert_verify").unwrap_or(false) {
        push_query_param(&mut params, "allowInsecure", Some("1".to_string()));
    }
    if let Some(alpn) = optional_string_list(node, "alpn") {
        push_query_param(&mut params, "alpn", Some(alpn.join(",")));
    }
    push_query_param(
        &mut params,
        "fp",
        optional_string(node, "client_fingerprint"),
    );
    Ok(build_uri_with_query(
        "vless",
        &percent_encode_userinfo(&uuid),
        node,
        &params,
    ))
}

fn build_trojan_uri(node: &ProxyNode) -> TransformResult<String> {
    let password = required_string(node, "password")?;
    let mut params = Vec::<(String, String)>::new();
    append_transport_params(node, &mut params);
    if node.tls.enabled {
        push_query_param(&mut params, "security", Some("tls".to_string()));
    }
    push_query_param(&mut params, "sni", node_server_name(node));
    if optional_bool(node, "skip_cert_verify").unwrap_or(false) {
        push_query_param(&mut params, "allowInsecure", Some("1".to_string()));
    }
    Ok(build_uri_with_query(
        "trojan",
        &percent_encode_userinfo(&password),
        node,
        &params,
    ))
}

fn build_anytls_uri(node: &ProxyNode) -> TransformResult<String> {
    let password = required_string(node, "password")?;
    let mut params = Vec::<(String, String)>::new();
    push_query_param(&mut params, "sni", node_server_name(node));
    if let Some(alpn) = optional_string_list(node, "alpn") {
        push_query_param(&mut params, "alpn", Some(alpn.join(",")));
    }
    push_query_param(
        &mut params,
        "fp",
        optional_string(node, "client_fingerprint"),
    );
    if optional_bool(node, "skip_cert_verify").unwrap_or(false) {
        push_query_param(&mut params, "allowInsecure", Some("1".to_string()));
    }
    Ok(build_uri_with_query(
        "anytls",
        &percent_encode_userinfo(&password),
        node,
        &params,
    ))
}

fn build_hysteria2_uri(node: &ProxyNode) -> TransformResult<String> {
    let auth = optional_string(node, "password")
        .or_else(|| optional_string(node, "auth"))
        .ok_or_else(|| TransformError::MissingField {
            node_name: node.name.clone(),
            field: "password/auth",
        })?;

    let mut params = Vec::<(String, String)>::new();
    push_query_param(&mut params, "obfs", optional_string(node, "obfs"));
    push_query_param(
        &mut params,
        "obfs-password",
        optional_string(node, "obfs_password"),
    );
    push_query_param(&mut params, "sni", node_server_name(node));
    if optional_bool(node, "skip_cert_verify").unwrap_or(false) {
        push_query_param(&mut params, "insecure", Some("1".to_string()));
    }
    if let Some(alpn) = optional_string_list(node, "alpn") {
        push_query_param(&mut params, "alpn", Some(alpn.join(",")));
    }
    Ok(build_uri_with_query(
        "hysteria2",
        &percent_encode_userinfo(&auth),
        node,
        &params,
    ))
}

fn build_tuic_uri(node: &ProxyNode) -> TransformResult<String> {
    let uuid = required_string(node, "uuid")?;
    let password = required_string(node, "password")?;
    let credentials = format!(
        "{}:{}",
        percent_encode_userinfo(&uuid),
        percent_encode_userinfo(&password)
    );

    let mut params = Vec::<(String, String)>::new();
    push_query_param(
        &mut params,
        "congestion_control",
        optional_string(node, "congestion_control"),
    );
    push_query_param(
        &mut params,
        "udp_relay_mode",
        optional_string(node, "udp_relay_mode"),
    );
    push_query_param(&mut params, "sni", node_server_name(node));
    if let Some(alpn) = optional_string_list(node, "alpn") {
        push_query_param(&mut params, "alpn", Some(alpn.join(",")));
    }
    if optional_bool(node, "skip_cert_verify").unwrap_or(false) {
        push_query_param(&mut params, "allow_insecure", Some("1".to_string()));
    }

    Ok(build_uri_with_query("tuic", &credentials, node, &params))
}

fn append_transport_params(node: &ProxyNode, params: &mut Vec<(String, String)>) {
    match node.transport {
        ProxyTransport::Tcp => {}
        ProxyTransport::Ws => {
            push_query_param(params, "type", Some("ws".to_string()));
            push_query_param(params, "host", optional_string(node, "host"));
            push_query_param(params, "path", optional_string(node, "path"));
        }
        ProxyTransport::Grpc => {
            push_query_param(params, "type", Some("grpc".to_string()));
            push_query_param(
                params,
                "serviceName",
                optional_string(node, "grpc_service_name")
                    .or_else(|| optional_string(node, "service_name")),
            );
        }
        ProxyTransport::H2 => {
            push_query_param(params, "type", Some("h2".to_string()));
            if let Some(hosts) = optional_string_list(node, "host") {
                push_query_param(params, "host", Some(hosts.join(",")));
            }
            push_query_param(params, "path", optional_string(node, "path"));
        }
        ProxyTransport::Quic => {
            push_query_param(params, "type", Some("quic".to_string()));
        }
    }
}

fn build_uri_with_query(
    scheme: &str,
    userinfo: &str,
    node: &ProxyNode,
    params: &[(String, String)],
) -> String {
    let mut uri = format!(
        "{scheme}://{userinfo}@{}:{}",
        format_host(&node.server),
        node.port
    );
    if !params.is_empty() {
        uri.push('?');
        uri.push_str(&encode_query_pairs(params));
    }
    uri.push('#');
    uri.push_str(&percent_encode_fragment(&node.name));
    uri
}

fn push_query_param(params: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(raw) = value {
        if !raw.is_empty() {
            params.push((key.to_string(), raw));
        }
    }
}

fn encode_query_pairs(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode_query_component(key),
                percent_encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode_userinfo(value: &str) -> String {
    percent_encode_component(value)
}

fn percent_encode_query_component(value: &str) -> String {
    percent_encode_component(value)
}

fn percent_encode_fragment(value: &str) -> String {
    percent_encode_component(value)
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(nibble_to_hex(byte >> 4));
            encoded.push(nibble_to_hex(byte & 0x0F));
        }
    }
    encoded
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + (value - 10)),
    }
}

fn format_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn node_server_name(node: &ProxyNode) -> Option<String> {
    node.tls
        .server_name
        .clone()
        .or_else(|| optional_string(node, "sni"))
}
