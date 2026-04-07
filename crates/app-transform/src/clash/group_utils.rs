use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use app_common::ProxyNode;

pub(super) fn collect_region_groups(nodes: &[ProxyNode]) -> BTreeMap<String, Vec<String>> {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    for node in nodes {
        let Some(region_name) = normalize_region_name(node.region.as_deref()) else {
            continue;
        };
        groups
            .entry(region_name)
            .or_default()
            .insert(node.name.clone());
    }

    groups
        .into_iter()
        .map(|(name, values)| (name, values.into_iter().collect::<Vec<_>>()))
        .collect()
}

pub(super) fn is_builtin_policy(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
    )
}

pub(super) fn filter_group_candidate_nodes(
    node_names: &[String],
    include_filter: Option<&str>,
    exclude_filter: Option<&str>,
) -> Vec<String> {
    node_names
        .iter()
        .filter(|name| matches_filter(name, include_filter, true))
        .filter(|name| matches_filter(name, exclude_filter, false))
        .cloned()
        .collect()
}

fn normalize_region_name(region: Option<&str>) -> Option<String> {
    let value = region?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_ascii_uppercase())
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
