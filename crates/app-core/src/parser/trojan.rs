use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport, TlsConfig};
use reqwest::Url;
use serde_json::Value;

use crate::CoreError;
use crate::CoreResult;

use super::{build_proxy_node, decode_percent_encoded};

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
    let name = decode_percent_encoded(
        url.fragment()
            .filter(|value| !value.is_empty())
            .unwrap_or("trojan"),
    );

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
