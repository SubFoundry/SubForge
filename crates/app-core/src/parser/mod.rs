use app_common::ProxyNode;

use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult};

mod base64;
mod common;
mod ss;
mod trojan;
mod vless;
mod vmess;

use base64::try_decode_base64_text;
use ss::parse_ss_uri;
use trojan::parse_trojan_uri;
use vless::parse_vless_uri;
use vmess::parse_vmess_uri;

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

pub(crate) use common::{
    build_proxy_node, decode_percent_encoded, map_transport, parse_host_port, split_fragment,
};
