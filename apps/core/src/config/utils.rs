use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

pub(super) fn resolve_path(base_dir: &Path, raw: &Path) -> Result<PathBuf> {
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base_dir.join(raw)
    };
    if joined.exists() {
        return joined
            .canonicalize()
            .with_context(|| format!("解析路径失败: {}", joined.display()));
    }
    if let Some(parent) = joined.parent() {
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .with_context(|| format!("解析路径失败: {}", parent.display()))?;
            if let Some(name) = joined.file_name() {
                return Ok(canonical_parent.join(name));
            }
        }
    }
    Ok(joined)
}

pub(super) fn toml_to_json_value(value: &TomlValue) -> Result<JsonValue> {
    serde_json::to_value(value).context("将 TOML 值转换为 JSON 失败")
}
