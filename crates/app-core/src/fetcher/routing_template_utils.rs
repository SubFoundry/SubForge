use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};

pub(super) fn parse_interval_seconds(value: Option<&JsonValue>) -> Option<u32> {
    let value = value?;
    if let Some(raw) = value.as_u64() {
        return u32::try_from(raw).ok();
    }

    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(raw) = text.parse::<u32>() {
        return Some(raw);
    }

    let unit = text.chars().last()?;
    let number = text[..text.len().saturating_sub(1)].trim();
    let value = number.parse::<u32>().ok()?;
    match unit {
        's' | 'S' => Some(value),
        'm' | 'M' => value.checked_mul(60),
        'h' | 'H' => value.checked_mul(60).and_then(|item| item.checked_mul(60)),
        _ => None,
    }
}

pub(super) fn parse_port_rule(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if let Some(raw) = value.as_u64() {
        return Some(raw.to_string());
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(super) fn push_prefixed_rules(
    output: &mut Vec<String>,
    rule: &JsonMap<String, JsonValue>,
    key: &str,
    prefix: &str,
    target: &str,
) {
    for value in json_string_values(rule.get(key)) {
        push_unique_rule(output, format!("{prefix},{value},{target}"));
    }
}

fn json_string_values(value: Option<&JsonValue>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return vec![text.to_string()];
    }

    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn push_unique_rule(output: &mut Vec<String>, value: String) {
    if !output.iter().any(|item| item == &value) {
        output.push(value);
    }
}

pub(super) fn yaml_map_get<'a>(mapping: &'a YamlMapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

pub(super) fn yaml_map_get_any<'a>(
    mapping: &'a YamlMapping,
    keys: &[&str],
) -> Option<&'a YamlValue> {
    keys.iter().find_map(|key| yaml_map_get(mapping, key))
}
