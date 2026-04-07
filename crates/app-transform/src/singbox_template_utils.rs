use regex::Regex;

use super::SingboxOutbound;

pub(super) fn build_builtin_outbounds(groups: &[SingboxOutbound]) -> Vec<SingboxOutbound> {
    let mut need_direct = false;
    let mut need_block = false;
    for group in groups {
        let Some(outbounds) = group.outbounds.as_ref() else {
            continue;
        };
        for item in outbounds {
            match item.as_str() {
                "direct" => need_direct = true,
                "block" => need_block = true,
                _ => {}
            }
        }
    }

    let mut builtins = Vec::new();
    if need_direct {
        builtins.push(simple_outbound("direct", "direct"));
    }
    if need_block {
        builtins.push(simple_outbound("block", "block"));
    }
    builtins
}

pub(super) fn normalize_policy_reference(name: &str) -> String {
    match name.trim().to_ascii_uppercase().as_str() {
        "DIRECT" | "PASS" | "COMPATIBLE" => "direct".to_string(),
        "REJECT" | "REJECT-DROP" => "block".to_string(),
        _ => name.to_string(),
    }
}

pub(super) fn is_builtin_policy(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
    )
}

pub(super) fn filter_group_candidate_tags(
    node_tags: &[String],
    include_filter: Option<&str>,
    exclude_filter: Option<&str>,
) -> Vec<String> {
    node_tags
        .iter()
        .filter(|tag| matches_filter(tag, include_filter, true))
        .filter(|tag| matches_filter(tag, exclude_filter, false))
        .cloned()
        .collect()
}

fn simple_outbound(outbound_type: &str, tag: &str) -> SingboxOutbound {
    SingboxOutbound {
        outbound_type: outbound_type.to_string(),
        tag: tag.to_string(),
        outbounds: None,
        default: None,
        url: None,
        interval: None,
        tolerance: None,
        server: None,
        server_port: None,
        method: None,
        password: None,
        uuid: None,
        security: None,
        alter_id: None,
        flow: None,
        network: None,
        tls: None,
        transport: None,
        obfs: None,
        congestion_control: None,
        udp_relay_mode: None,
    }
}

fn matches_filter(value: &str, pattern: Option<&str>, include_mode: bool) -> bool {
    let Some(pattern) = pattern.map(str::trim).filter(|item| !item.is_empty()) else {
        return true;
    };

    let matched = Regex::new(pattern)
        .map(|regex| regex.is_match(value))
        .unwrap_or_else(|_| value.contains(pattern));
    if include_mode { matched } else { !matched }
}
