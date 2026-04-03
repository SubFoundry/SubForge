use std::collections::{BTreeMap, BTreeSet};

use app_common::{Profile, ProxyNode};
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
        let mut proxies = Vec::with_capacity(nodes.len());
        for node in nodes {
            proxies.push(proxy::build_clash_proxy(node)?);
        }

        let config = ClashConfig {
            proxies,
            proxy_groups: self.build_proxy_groups(nodes),
        };
        Ok(serde_yaml::to_string(&config)?)
    }
}

impl ClashTransformer {
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

#[derive(Debug, Serialize)]
struct ClashConfig {
    proxies: Vec<ClashProxy>,
    #[serde(rename = "proxy-groups")]
    proxy_groups: Vec<ClashProxyGroup>,
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
