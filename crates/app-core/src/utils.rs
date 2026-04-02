use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use app_common::ConfigSchemaProperty;
use app_plugin_runtime::LoadedPlugin;
use base64::Engine as Base64Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD;
use regex::Regex;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{CoreError, CoreResult, SECRET_PLACEHOLDER};

pub(crate) fn now_rfc3339() -> CoreResult<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

pub(crate) fn generate_secure_token() -> CoreResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(BASE64_URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn plugin_scope(plugin_id: &str) -> String {
    format!("plugin:{plugin_id}")
}

pub(crate) fn is_scalar_json(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

pub(crate) fn masked_config(
    loaded: &LoadedPlugin,
    normalized_config: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut result = normalized_config.clone();
    for key in &loaded.manifest.secret_fields {
        if result.contains_key(key) {
            result.insert(key.clone(), Value::String(SECRET_PLACEHOLDER.to_string()));
        }
    }
    result
}

pub(crate) fn validate_property_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    value: &Value,
) -> CoreResult<Value> {
    let mut validated = match property.property_type.as_str() {
        "string" => {
            let text = value.as_str().ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 string"))
            })?;
            if let Some(min_length) = property.min_length {
                if text.chars().count() < min_length as usize {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 长度不能小于 {min_length}"
                    )));
                }
            }
            if let Some(max_length) = property.max_length {
                if text.chars().count() > max_length as usize {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 长度不能大于 {max_length}"
                    )));
                }
            }
            if let Some(pattern) = &property.pattern {
                let regex = Regex::new(pattern).map_err(|error| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 的 pattern 非法：{error}"))
                })?;
                if !regex.is_match(text) {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不匹配 pattern 约束"
                    )));
                }
            }
            Value::String(text.to_string())
        }
        "number" => {
            let number = value.as_f64().ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 number"))
            })?;
            if let Some(minimum) = property.minimum {
                if number < minimum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能小于 {minimum}"
                    )));
                }
            }
            if let Some(maximum) = property.maximum {
                if number > maximum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能大于 {maximum}"
                    )));
                }
            }
            let parsed = serde_json::Number::from_f64(number).ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 不是有效 number"))
            })?;
            Value::Number(parsed)
        }
        "integer" => {
            let parsed = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
                .ok_or_else(|| {
                    CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 integer"))
                })?;
            if let Some(minimum) = property.minimum {
                if (parsed as f64) < minimum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能小于 {minimum}"
                    )));
                }
            }
            if let Some(maximum) = property.maximum {
                if (parsed as f64) > maximum {
                    return Err(CoreError::ConfigInvalid(format!(
                        "字段 {field_name} 不能大于 {maximum}"
                    )));
                }
            }
            Value::Number(parsed.into())
        }
        "boolean" => Value::Bool(value.as_bool().ok_or_else(|| {
            CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 boolean"))
        })?),
        _ => {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 包含不支持类型：{}",
                property.property_type
            )));
        }
    };

    if let Some(enum_values) = &property.enum_values {
        if !enum_values.iter().any(|item| item == &validated) {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 必须在枚举值范围内"
            )));
        }
    }

    if property.property_type == "integer" {
        // 统一整数 JSON 形态，避免 1 和 1.0 在枚举比较时产生歧义。
        if let Some(raw) = validated.as_i64() {
            validated = Value::Number(raw.into());
        }
    }

    Ok(validated)
}

pub(crate) fn stringify_secret_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    value: &Value,
) -> CoreResult<String> {
    match property.property_type.as_str() {
        "string" => value
            .as_str()
            .map(ToString::to_string)
            .ok_or_else(|| CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 string"))),
        "number" | "integer" => Ok(value
            .as_i64()
            .map(|raw| raw.to_string())
            .or_else(|| value.as_u64().map(|raw| raw.to_string()))
            .or_else(|| value.as_f64().map(|raw| raw.to_string()))
            .ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 number/integer"))
            })?),
        "boolean" => value
            .as_bool()
            .map(|raw| raw.to_string())
            .ok_or_else(|| CoreError::ConfigInvalid(format!("字段 {field_name} 必须是 boolean"))),
        _ => Err(CoreError::ConfigInvalid(format!(
            "字段 {field_name} 包含不支持类型：{}",
            property.property_type
        ))),
    }
}

pub(crate) fn inflate_typed_value(
    field_name: &str,
    property: &ConfigSchemaProperty,
    raw: &str,
) -> CoreResult<Value> {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        return validate_property_value(field_name, property, &parsed);
    }

    // 兼容旧存储格式（value 直接保存为字符串）。
    let fallback = match property.property_type.as_str() {
        "string" => Value::String(raw.to_string()),
        "number" => {
            let parsed = raw.parse::<f64>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} number 解析失败：{error}"))
            })?;
            Value::Number(serde_json::Number::from_f64(parsed).ok_or_else(|| {
                CoreError::ConfigInvalid(format!("字段 {field_name} 不是有效 number"))
            })?)
        }
        "integer" => {
            let parsed = raw.parse::<i64>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} integer 解析失败：{error}"))
            })?;
            Value::Number(parsed.into())
        }
        "boolean" => {
            let parsed = raw.parse::<bool>().map_err(|error| {
                CoreError::ConfigInvalid(format!("字段 {field_name} boolean 解析失败：{error}"))
            })?;
            Value::Bool(parsed)
        }
        _ => {
            return Err(CoreError::ConfigInvalid(format!(
                "字段 {field_name} 包含不支持类型：{}",
                property.property_type
            )));
        }
    };

    validate_property_value(field_name, property, &fallback)
}

pub(crate) fn copy_dir_recursive(source: &Path, target: &Path) -> CoreResult<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}
