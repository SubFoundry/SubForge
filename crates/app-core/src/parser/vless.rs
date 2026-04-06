use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use reqwest::Url;
use serde_json::Value;

use crate::CoreError;
use crate::CoreResult;

use super::{build_proxy_node, decode_percent_encoded, map_transport};

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
    let name = decode_percent_encoded(
        url.fragment()
            .filter(|value| !value.is_empty())
            .unwrap_or("vless"),
    );
    let transport = map_transport(url.query_pairs().find_map(|(k, v)| {
        if k == "type" {
            Some(v.to_string())
        } else {
            None
        }
    }));
    let query_pairs = url.query_pairs().collect::<Vec<_>>();
    let security = query_value(&query_pairs, &["security"]);
    let sni = query_value(&query_pairs, &["sni", "peer", "servername"]);

    let mut extra = BTreeMap::new();
    if !url.username().is_empty() {
        extra.insert(
            "uuid".to_string(),
            Value::String(url.username().to_string()),
        );
    }
    insert_optional_string(
        &mut extra,
        "security",
        query_value(&query_pairs, &["security"]),
    );
    insert_optional_string(&mut extra, "flow", query_value(&query_pairs, &["flow"]));
    insert_optional_string(
        &mut extra,
        "service_name",
        query_value(&query_pairs, &["serviceName", "service_name"]),
    );
    insert_optional_string(
        &mut extra,
        "grpc_service_name",
        query_value(&query_pairs, &["grpc_service_name"]),
    );
    insert_optional_string(
        &mut extra,
        "client_fingerprint",
        query_value(&query_pairs, &["fp", "fingerprint"]),
    );
    insert_optional_bool(
        &mut extra,
        "skip_cert_verify",
        query_bool(
            &query_pairs,
            &["allowInsecure", "allow_insecure", "insecure"],
        ),
    );
    insert_optional_alpn(&mut extra, query_value(&query_pairs, &["alpn", "alpns"]));
    insert_transport_host(
        &mut extra,
        query_value(&query_pairs, &["host"]),
        transport.clone(),
    );
    insert_optional_string(&mut extra, "path", query_value(&query_pairs, &["path"]));

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

fn insert_transport_host(
    extra: &mut BTreeMap<String, Value>,
    host: Option<String>,
    transport: ProxyTransport,
) {
    let Some(host) = host else {
        return;
    };
    let hosts = split_csv_values(&host);
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

fn split_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}
