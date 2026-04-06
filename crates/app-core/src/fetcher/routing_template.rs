use app_common::{
    ClashRoutingTemplate, RoutingTemplateGroupIr, RoutingTemplateIr, RoutingTemplateSourceKernel,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::Value as YamlValue;

#[path = "routing_template_utils.rs"]
mod utils;
use utils::{
    parse_interval_seconds, parse_port_rule, push_prefixed_rules, push_unique_rule, yaml_map_get,
    yaml_map_get_any,
};

pub(super) fn source_routing_template_key(source_instance_id: &str) -> String {
    format!("source.{source_instance_id}.clash_routing_template")
}

pub(super) fn extract_clash_routing_template(payload: &str) -> Option<ClashRoutingTemplate> {
    parse_routing_template_ir(payload).map(RoutingTemplateIr::into_clash_template)
}

fn parse_routing_template_ir(payload: &str) -> Option<RoutingTemplateIr> {
    parse_clash_routing_template_ir(payload).or_else(|| parse_singbox_routing_template_ir(payload))
}

fn parse_clash_routing_template_ir(payload: &str) -> Option<RoutingTemplateIr> {
    let root = serde_yaml::from_str::<YamlValue>(payload).ok()?;
    let root = root.as_mapping()?;
    let groups_value = yaml_map_get(root, "proxy-groups")?;
    let groups = groups_value.as_sequence()?;

    let mut parsed_groups = Vec::new();
    for group in groups {
        let Some(group_map) = group.as_mapping() else {
            continue;
        };
        let Some(name) = yaml_map_get(group_map, "name")
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(group_type) = yaml_map_get(group_map, "type")
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let proxies = yaml_map_get(group_map, "proxies")
            .and_then(YamlValue::as_sequence)
            .map(|items| {
                items
                    .iter()
                    .filter_map(YamlValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let url = yaml_map_get(group_map, "url")
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let interval = yaml_map_get(group_map, "interval")
            .and_then(YamlValue::as_i64)
            .and_then(|value| u32::try_from(value).ok());
        let tolerance = yaml_map_get(group_map, "tolerance")
            .and_then(YamlValue::as_i64)
            .and_then(|value| u16::try_from(value).ok());
        let include_all = yaml_map_get_any(
            group_map,
            &[
                "include-all",
                "include_all",
                "include-all-proxies",
                "include_all_proxies",
                "include-all-providers",
                "include_all_providers",
            ],
        )
        .and_then(YamlValue::as_bool)
        .unwrap_or(false);
        let use_provider = yaml_map_get(group_map, "use")
            .and_then(YamlValue::as_sequence)
            .is_some_and(|items| !items.is_empty());
        let filter = yaml_map_get(group_map, "filter")
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let exclude_filter = yaml_map_get_any(group_map, &["exclude-filter", "exclude_filter"])
            .and_then(YamlValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        parsed_groups.push(RoutingTemplateGroupIr {
            name: name.to_string(),
            group_type: group_type.to_string(),
            proxies,
            url,
            interval,
            tolerance,
            include_all,
            use_provider,
            filter,
            exclude_filter,
        });
    }

    if parsed_groups.is_empty() {
        return None;
    }

    let rules = yaml_map_get(root, "rules")
        .and_then(YamlValue::as_sequence)
        .map(|items| {
            items
                .iter()
                .filter_map(YamlValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(RoutingTemplateIr {
        groups: parsed_groups,
        rules,
        source_kernel: RoutingTemplateSourceKernel::Clash,
        meta: None,
    })
}

fn parse_singbox_routing_template_ir(payload: &str) -> Option<RoutingTemplateIr> {
    let root = serde_json::from_str::<JsonValue>(payload).ok()?;
    let outbounds = root.get("outbounds")?.as_array()?;

    let mut groups = Vec::new();
    for outbound in outbounds {
        let Some(group) = parse_singbox_group(outbound) else {
            continue;
        };
        groups.push(group);
    }

    if groups.is_empty() {
        return None;
    }

    let rules = parse_singbox_rules(root.get("route"));
    Some(RoutingTemplateIr {
        groups,
        rules,
        source_kernel: RoutingTemplateSourceKernel::SingBox,
        meta: None,
    })
}

fn parse_singbox_group(outbound: &JsonValue) -> Option<RoutingTemplateGroupIr> {
    let map = outbound.as_object()?;
    let group_type = map.get("type")?.as_str()?.trim();
    let mapped_group_type = match group_type {
        "selector" => "select",
        "urltest" => "url-test",
        _ => return None,
    };

    let name = map
        .get("tag")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let proxies = map
        .get("outbounds")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let url = map
        .get("url")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let interval = parse_interval_seconds(map.get("interval"));
    let tolerance = map
        .get("tolerance")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok());

    Some(RoutingTemplateGroupIr {
        name: name.to_string(),
        group_type: mapped_group_type.to_string(),
        proxies,
        url,
        interval,
        tolerance,
        include_all: false,
        use_provider: false,
        filter: None,
        exclude_filter: None,
    })
}

fn parse_singbox_rules(route: Option<&JsonValue>) -> Vec<String> {
    let Some(route_map) = route.and_then(JsonValue::as_object) else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    if let Some(items) = route_map.get("rules").and_then(JsonValue::as_array) {
        for item in items {
            let Some(rule_map) = item.as_object() else {
                continue;
            };
            let Some(target) = resolve_singbox_rule_target(rule_map) else {
                continue;
            };

            push_prefixed_rules(&mut rules, rule_map, "domain", "DOMAIN", &target);
            push_prefixed_rules(
                &mut rules,
                rule_map,
                "domain_suffix",
                "DOMAIN-SUFFIX",
                &target,
            );
            push_prefixed_rules(
                &mut rules,
                rule_map,
                "domain_keyword",
                "DOMAIN-KEYWORD",
                &target,
            );
            push_prefixed_rules(&mut rules, rule_map, "ip_cidr", "IP-CIDR", &target);
            push_prefixed_rules(&mut rules, rule_map, "geoip", "GEOIP", &target);
            push_prefixed_rules(&mut rules, rule_map, "geosite", "GEOSITE", &target);
            push_prefixed_rules(&mut rules, rule_map, "network", "NETWORK", &target);

            if let Some(port) = parse_port_rule(rule_map.get("port")) {
                push_unique_rule(&mut rules, format!("DST-PORT,{port},{target}"));
            }
        }
    }

    if let Some(final_target) = route_map
        .get("final")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_unique_rule(&mut rules, format!("MATCH,{final_target}"));
    }

    rules
}

fn resolve_singbox_rule_target(rule: &JsonMap<String, JsonValue>) -> Option<String> {
    if let Some(target) = rule
        .get("outbound")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(target.to_string());
    }

    let first = rule
        .get("outbounds")
        .and_then(JsonValue::as_array)
        .and_then(|items| items.first())
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(first.to_string())
}
