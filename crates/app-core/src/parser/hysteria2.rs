use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use reqwest::Url;
use serde_json::Value;

use crate::CoreError;
use crate::CoreResult;

use super::{build_proxy_node, decode_percent_encoded};

pub(crate) fn parse_hysteria2_uri(
    line: &str,
    source_id: &str,
    updated_at: &str,
) -> CoreResult<ProxyNode> {
    let url = Url::parse(line)
        .map_err(|error| CoreError::SubscriptionParse(format!("hysteria2 URI 非法：{error}")))?;
    let server = url
        .host_str()
        .ok_or_else(|| CoreError::SubscriptionParse("hysteria2 URI 缺少 host".to_string()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CoreError::SubscriptionParse("hysteria2 URI 缺少端口".to_string()))?;
    let name = decode_percent_encoded(
        url.fragment()
            .filter(|value| !value.is_empty())
            .unwrap_or("hysteria2"),
    );

    let query_pairs = url.query_pairs().collect::<Vec<_>>();
    let mut extra = BTreeMap::new();
    let auth = if !url.username().is_empty() {
        Some(url.username().to_string())
    } else {
        None
    };
    insert_optional_string(&mut extra, "password", auth.clone());
    insert_optional_string(&mut extra, "auth", auth);
    insert_optional_string(&mut extra, "obfs", query_value(&query_pairs, &["obfs"]));
    insert_optional_string(
        &mut extra,
        "obfs_password",
        query_value(&query_pairs, &["obfs-password", "obfs_password"]),
    );
    insert_optional_bool(
        &mut extra,
        "skip_cert_verify",
        query_bool(&query_pairs, &["insecure", "allowInsecure", "allow_insecure"]),
    );
    insert_optional_alpn(&mut extra, query_value(&query_pairs, &["alpn", "alpns"]));

    let server_name = query_value(&query_pairs, &["sni", "peer", "servername"]);

    Ok(build_proxy_node(
        source_id,
        name,
        ProxyProtocol::Hysteria2,
        server,
        port,
        ProxyTransport::Quic,
        TlsConfig {
            enabled: true,
            server_name,
        },
        extra,
        updated_at,
    ))
}

fn query_value(
    query_pairs: &[(std::borrow::Cow<'_, str>, std::borrow::Cow<'_, str>)],
    keys: &[&str],
) -> Option<String> {
    query_pairs
        .iter()
        .find_map(|(key, value)| {
            if keys
                .iter()
                .any(|candidate| key.eq_ignore_ascii_case(candidate))
            {
                Some(value.trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
}

fn query_bool(
    query_pairs: &[(std::borrow::Cow<'_, str>, std::borrow::Cow<'_, str>)],
    keys: &[&str],
) -> Option<bool> {
    query_value(query_pairs, keys).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    })
}

fn insert_optional_string(extra: &mut BTreeMap<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extra.insert(key.to_string(), Value::String(value));
    }
}

fn insert_optional_bool(extra: &mut BTreeMap<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        extra.insert(key.to_string(), Value::Bool(value));
    }
}

fn insert_optional_alpn(extra: &mut BTreeMap<String, Value>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let values = split_csv_values(&value);
    if values.is_empty() {
        return;
    }
    extra.insert(
        "alpn".to_string(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
}

fn split_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}
