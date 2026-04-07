use app_common::{ClashRoutingTemplate, Profile, ProxyNode};
use serde::Serialize;

use crate::shared::push_unique_proxy_name;
use crate::{TransformResult, Transformer};

#[path = "singbox_outbound.rs"]
mod outbound;
#[path = "singbox_template_utils.rs"]
mod template_utils;
use outbound::{SingboxObfs, SingboxTls, SingboxTransport, build_singbox_node_outbound};
use template_utils::{
    build_builtin_outbounds, filter_group_candidate_tags, is_builtin_policy,
    normalize_policy_reference,
};

/// sing-box JSON 转换器。
#[derive(Debug, Clone)]
pub struct SingboxTransformer {
    auto_test_url: String,
    auto_test_interval: String,
    auto_test_tolerance: u16,
}

impl Default for SingboxTransformer {
    fn default() -> Self {
        Self {
            auto_test_url: "https://www.gstatic.com/generate_204".to_string(),
            auto_test_interval: "5m".to_string(),
            auto_test_tolerance: 50,
        }
    }
}

impl Transformer for SingboxTransformer {
    fn transform(&self, nodes: &[ProxyNode], _profile: &Profile) -> TransformResult<String> {
        self.transform_with_template(nodes, None)
    }
}

impl SingboxTransformer {
    pub fn transform_with_template(
        &self,
        nodes: &[ProxyNode],
        routing_template: Option<&ClashRoutingTemplate>,
    ) -> TransformResult<String> {
        let mut node_tags = Vec::with_capacity(nodes.len());
        let mut node_outbounds = Vec::with_capacity(nodes.len());
        for node in nodes {
            node_tags.push(node.name.clone());
            node_outbounds.push(build_singbox_node_outbound(node)?);
        }

        let mut group_outbounds = match routing_template {
            Some(template) => self.build_template_groups(nodes, template),
            None => self.build_default_groups(&node_tags),
        };
        let mut builtin_outbounds = build_builtin_outbounds(&group_outbounds);

        let mut outbounds = Vec::with_capacity(
            builtin_outbounds.len() + group_outbounds.len() + node_outbounds.len(),
        );
        outbounds.append(&mut builtin_outbounds);
        outbounds.append(&mut group_outbounds);
        outbounds.extend(node_outbounds);

        let config = SingboxConfig { outbounds };
        Ok(serde_json::to_string_pretty(&config)?)
    }

    fn build_default_groups(&self, node_tags: &[String]) -> Vec<SingboxOutbound> {
        let mut selector_targets = Vec::with_capacity(node_tags.len() + 1);
        push_unique_proxy_name(&mut selector_targets, "auto");
        for tag in node_tags {
            push_unique_proxy_name(&mut selector_targets, tag);
        }

        vec![
            SingboxOutbound {
                outbound_type: "selector".to_string(),
                tag: "select".to_string(),
                outbounds: Some(selector_targets),
                default: Some("auto".to_string()),
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
            },
            SingboxOutbound {
                outbound_type: "urltest".to_string(),
                tag: "auto".to_string(),
                outbounds: Some(node_tags.to_vec()),
                default: None,
                url: Some(self.auto_test_url.clone()),
                interval: Some(self.auto_test_interval.clone()),
                tolerance: Some(self.auto_test_tolerance),
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
            },
        ]
    }

    fn build_template_groups(
        &self,
        nodes: &[ProxyNode],
        routing_template: &ClashRoutingTemplate,
    ) -> Vec<SingboxOutbound> {
        let aggregated_node_tags = nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let group_name_set = routing_template
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        let mut groups = Vec::with_capacity(routing_template.groups.len());
        for template_group in &routing_template.groups {
            let candidate_tags = filter_group_candidate_tags(
                &aggregated_node_tags,
                template_group.filter.as_deref(),
                template_group.exclude_filter.as_deref(),
            );
            let has_plain_node_slot = template_group
                .proxies
                .iter()
                .any(|item| !group_name_set.contains(item.as_str()) && !is_builtin_policy(item));
            let should_append_nodes = has_plain_node_slot
                || (template_group.proxies.is_empty()
                    && (template_group.include_all || template_group.use_provider));

            let mut targets = Vec::new();
            if routing_template.preserve_original_proxy_names {
                for item in &template_group.proxies {
                    push_unique_proxy_name(&mut targets, &normalize_policy_reference(item));
                }
                if should_append_nodes {
                    for tag in &candidate_tags {
                        push_unique_proxy_name(&mut targets, tag);
                    }
                }
            } else {
                let mut inserted_aggregated = false;
                for item in &template_group.proxies {
                    if group_name_set.contains(item.as_str()) || is_builtin_policy(item) {
                        push_unique_proxy_name(&mut targets, &normalize_policy_reference(item));
                        continue;
                    }
                    if !inserted_aggregated {
                        for tag in &candidate_tags {
                            push_unique_proxy_name(&mut targets, tag);
                        }
                        inserted_aggregated = true;
                    }
                }
                if !inserted_aggregated && should_append_nodes {
                    for tag in &candidate_tags {
                        push_unique_proxy_name(&mut targets, tag);
                    }
                }
            }

            let is_urltest = matches!(
                template_group
                    .group_type
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "url-test" | "urltest"
            );
            groups.push(SingboxOutbound {
                outbound_type: if is_urltest {
                    "urltest".to_string()
                } else {
                    "selector".to_string()
                },
                tag: template_group.name.clone(),
                outbounds: Some(targets),
                default: None,
                url: is_urltest.then(|| {
                    template_group
                        .url
                        .clone()
                        .unwrap_or_else(|| self.auto_test_url.clone())
                }),
                interval: is_urltest.then(|| {
                    template_group
                        .interval
                        .map(|value| format!("{value}s"))
                        .unwrap_or_else(|| self.auto_test_interval.clone())
                }),
                tolerance: is_urltest
                    .then_some(template_group.tolerance.unwrap_or(self.auto_test_tolerance)),
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
            });
        }

        if groups.is_empty() {
            self.build_default_groups(&aggregated_node_tags)
        } else {
            groups
        }
    }
}

#[derive(Debug, Serialize)]
struct SingboxConfig {
    outbounds: Vec<SingboxOutbound>,
}

#[derive(Debug, Serialize)]
pub(super) struct SingboxOutbound {
    #[serde(rename = "type")]
    outbound_type: String,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    outbounds: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tolerance: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(rename = "server_port", skip_serializing_if = "Option::is_none")]
    server_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    security: Option<String>,
    #[serde(rename = "alter_id", skip_serializing_if = "Option::is_none")]
    alter_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<SingboxTls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<SingboxTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obfs: Option<SingboxObfs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    congestion_control: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    udp_relay_mode: Option<String>,
}
