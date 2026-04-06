use std::collections::{BTreeMap, BTreeSet};

use app_common::{ClashRoutingTemplate, Profile, ProxyNode};
use regex::Regex;
use serde::Serialize;

use crate::shared::push_unique_proxy_name;
use crate::{TransformResult, Transformer};

mod proxy;

/// Clash/Mihomo YAML 转换器。
#[derive(Debug, Clone)]
pub struct ClashTransformer {
    auto_test_url: String,
    auto_test_interval_seconds: u32,
    auto_test_tolerance: u16,
}

impl Default for ClashTransformer {
    fn default() -> Self {
        Self {
            auto_test_url: "http://www.gstatic.com/generate_204".to_string(),
            auto_test_interval_seconds: 300,
            auto_test_tolerance: 50,
        }
    }
}

impl Transformer for ClashTransformer {
    fn transform(&self, nodes: &[ProxyNode], _profile: &Profile) -> TransformResult<String> {
        self.transform_with_template(nodes, None)
    }
}

impl ClashTransformer {
    pub fn transform_with_template(
        &self,
        nodes: &[ProxyNode],
        routing_template: Option<&ClashRoutingTemplate>,
    ) -> TransformResult<String> {
        let mut proxies = Vec::with_capacity(nodes.len());
        for node in nodes {
            proxies.push(proxy::build_clash_proxy(node)?);
        }

        let (proxy_groups, template_applied) = match routing_template {
            Some(template) => self.build_proxy_groups_from_template(nodes, template),
            None => (self.build_proxy_groups(nodes), false),
        };
        let rules = if template_applied {
            routing_template.and_then(|template| {
                if template.rules.is_empty() {
                    None
                } else {
                    Some(template.rules.clone())
                }
            })
        } else {
            None
        };

        let config = ClashConfig {
            proxies,
            proxy_groups,
            rules,
        };
        Ok(serde_yaml::to_string(&config)?)
    }

    fn build_proxy_groups(&self, nodes: &[ProxyNode]) -> Vec<ClashProxyGroup> {
        let node_names = nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let region_groups = collect_region_groups(nodes);

        let mut select_proxies = Vec::new();
        push_unique_proxy_name(&mut select_proxies, "Auto");
        for region_name in region_groups.keys() {
            push_unique_proxy_name(&mut select_proxies, region_name);
        }
        for node_name in &node_names {
            push_unique_proxy_name(&mut select_proxies, node_name);
        }

        let mut groups = vec![
            ClashProxyGroup {
                name: "Select".to_string(),
                group_type: "select".to_string(),
                proxies: select_proxies,
                url: None,
                interval: None,
                tolerance: None,
            },
            ClashProxyGroup {
                name: "Auto".to_string(),
                group_type: "url-test".to_string(),
                proxies: node_names,
                url: Some(self.auto_test_url.clone()),
                interval: Some(self.auto_test_interval_seconds),
                tolerance: Some(self.auto_test_tolerance),
            },
        ];

        for (region_name, region_node_names) in region_groups {
            groups.push(ClashProxyGroup {
                name: region_name,
                group_type: "select".to_string(),
                proxies: region_node_names,
                url: None,
                interval: None,
                tolerance: None,
            });
        }

        groups
    }

    fn build_proxy_groups_from_template(
        &self,
        nodes: &[ProxyNode],
        routing_template: &ClashRoutingTemplate,
    ) -> (Vec<ClashProxyGroup>, bool) {
        let aggregated_node_names = nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let group_name_set = routing_template
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<BTreeSet<_>>();

        let mut groups = Vec::with_capacity(routing_template.groups.len());
        let mut injected_any_nodes = false;
        for template_group in &routing_template.groups {
            let mut proxies = Vec::new();
            let mut inserted_aggregated_nodes = false;
            let mut has_plain_node_slot = false;
            let candidate_nodes = filter_group_candidate_nodes(
                &aggregated_node_names,
                template_group.filter.as_deref(),
                template_group.exclude_filter.as_deref(),
            );

            for item in &template_group.proxies {
                if group_name_set.contains(item.as_str()) || is_builtin_policy(item) {
                    push_unique_proxy_name(&mut proxies, item);
                    continue;
                }
                has_plain_node_slot = true;
                if !inserted_aggregated_nodes {
                    for name in &candidate_nodes {
                        push_unique_proxy_name(&mut proxies, name);
                    }
                    inserted_aggregated_nodes = true;
                }
            }

            let should_inject_without_slot =
                template_group.include_all || template_group.use_provider;
            if !inserted_aggregated_nodes
                && !candidate_nodes.is_empty()
                && (has_plain_node_slot
                    || template_group.proxies.is_empty()
                    || should_inject_without_slot)
            {
                for name in &candidate_nodes {
                    push_unique_proxy_name(&mut proxies, name);
                }
                inserted_aggregated_nodes = true;
            }

            if inserted_aggregated_nodes {
                injected_any_nodes = true;
            }

            groups.push(ClashProxyGroup {
                name: template_group.name.clone(),
                group_type: template_group.group_type.clone(),
                proxies,
                url: template_group.url.clone(),
                interval: template_group.interval,
                tolerance: template_group.tolerance,
            });
        }

        if !injected_any_nodes
            && !aggregated_node_names.is_empty()
            && let Some(first_group) = groups.first_mut()
        {
            for name in &aggregated_node_names {
                push_unique_proxy_name(&mut first_group.proxies, name);
            }
        }

        if groups.is_empty() {
            (self.build_proxy_groups(nodes), false)
        } else {
            (groups, true)
        }
    }
}

fn collect_region_groups(nodes: &[ProxyNode]) -> BTreeMap<String, Vec<String>> {
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

fn normalize_region_name(region: Option<&str>) -> Option<String> {
    let value = region?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_ascii_uppercase())
    }
}

fn is_builtin_policy(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
    )
}

fn filter_group_candidate_nodes(
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

fn matches_filter(value: &str, pattern: Option<&str>, include_mode: bool) -> bool {
    let Some(pattern) = pattern.map(str::trim).filter(|item| !item.is_empty()) else {
        return true;
    };

    let matched = Regex::new(pattern)
        .map(|regex| regex.is_match(value))
        .unwrap_or_else(|_| value.contains(pattern));
    if include_mode { matched } else { !matched }
}

#[derive(Debug, Serialize)]
struct ClashConfig {
    proxies: Vec<ClashProxy>,
    #[serde(rename = "proxy-groups")]
    proxy_groups: Vec<ClashProxyGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rules: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ClashProxyGroup {
    name: String,
    #[serde(rename = "type")]
    group_type: String,
    proxies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tolerance: Option<u16>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClashProxy {
    name: String,
    #[serde(rename = "type")]
    proxy_type: String,
    server: String,
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    cipher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
    #[serde(rename = "alterId", skip_serializing_if = "Option::is_none")]
    alter_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    udp: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sni: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    servername: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<String>,
    #[serde(rename = "skip-cert-verify", skip_serializing_if = "Option::is_none")]
    skip_cert_verify: Option<bool>,
    #[serde(rename = "client-fingerprint", skip_serializing_if = "Option::is_none")]
    client_fingerprint: Option<String>,
    #[serde(rename = "ws-opts", skip_serializing_if = "Option::is_none")]
    ws_opts: Option<ClashWsOptions>,
    #[serde(rename = "grpc-opts", skip_serializing_if = "Option::is_none")]
    grpc_opts: Option<ClashGrpcOptions>,
    #[serde(rename = "h2-opts", skip_serializing_if = "Option::is_none")]
    h2_opts: Option<ClashH2Options>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alpn: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obfs: Option<String>,
    #[serde(rename = "obfs-password", skip_serializing_if = "Option::is_none")]
    obfs_password: Option<String>,
    #[serde(
        rename = "congestion-controller",
        skip_serializing_if = "Option::is_none"
    )]
    congestion_control: Option<String>,
    #[serde(rename = "udp-relay-mode", skip_serializing_if = "Option::is_none")]
    udp_relay_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClashWsOptions {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(rename = "max-early-data", skip_serializing_if = "Option::is_none")]
    max_early_data: Option<u32>,
    #[serde(
        rename = "early-data-header-name",
        skip_serializing_if = "Option::is_none"
    )]
    early_data_header_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClashGrpcOptions {
    #[serde(rename = "grpc-service-name")]
    grpc_service_name: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ClashH2Options {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}
