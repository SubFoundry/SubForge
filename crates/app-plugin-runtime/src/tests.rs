use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{PluginLoader, PluginRuntimeError};

#[test]
fn load_builtin_static_plugin_successfully() {
    let loader = PluginLoader::new();
    let path = builtins_static_plugin_dir();

    let loaded = loader
        .load_from_dir(&path)
        .expect("内置静态插件应能成功加载");

    assert_eq!(loaded.manifest.plugin_id, "subforge.builtin.static");
    assert_eq!(loaded.manifest.spec_version, "1.0");
    assert!(loaded.schema.properties.contains_key("url"));
}

#[test]
fn reject_missing_plugin_json() {
    let temp_root = create_temp_plugin_dir("missing-plugin-json");
    fs::write(temp_root.join("schema.json"), "{}").expect("写入 schema 失败");

    let loader = PluginLoader::new();
    let error = loader
        .load_from_dir(&temp_root)
        .expect_err("缺失 plugin.json 时应失败");

    assert!(matches!(error, PluginRuntimeError::Io(_)));
    cleanup_dir(&temp_root);
}

#[test]
fn reject_incompatible_spec_version() {
    let temp_root = create_temp_plugin_dir("invalid-spec-version");
    fs::write(
        temp_root.join("plugin.json"),
        r#"{
            "plugin_id": "vendor.example.static",
            "spec_version": "2.0",
            "name": "Broken",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json"
        }"#,
    )
    .expect("写入 plugin.json 失败");
    fs::write(
        temp_root.join("schema.json"),
        r#"{
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": { "type": "string" }
            }
        }"#,
    )
    .expect("写入 schema.json 失败");

    let loader = PluginLoader::new();
    let error = loader
        .load_from_dir(&temp_root)
        .expect_err("spec_version=2.0 时应拒绝");

    assert!(matches!(error, PluginRuntimeError::Incompatible(_)));
    cleanup_dir(&temp_root);
}

#[test]
fn reject_secret_field_without_schema_definition() {
    let temp_root = create_temp_plugin_dir("secret-mismatch");
    fs::write(
        temp_root.join("plugin.json"),
        r#"{
            "plugin_id": "vendor.example.script",
            "spec_version": "1.0",
            "name": "Secret Mismatch",
            "version": "1.0.0",
            "type": "script",
            "config_schema": "schema.json",
            "secret_fields": ["password"],
            "entrypoints": { "fetch": "scripts/fetch.lua" },
            "capabilities": ["http"]
        }"#,
    )
    .expect("写入 plugin.json 失败");
    fs::write(
        temp_root.join("schema.json"),
        r#"{
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": { "type": "string" }
            }
        }"#,
    )
    .expect("写入 schema.json 失败");

    let loader = PluginLoader::new();
    let error = loader
        .load_from_dir(&temp_root)
        .expect_err("secret_fields 不匹配时应失败");

    assert!(matches!(error, PluginRuntimeError::Invalid(_)));
    cleanup_dir(&temp_root);
}

#[test]
fn reject_unsupported_schema_keyword() {
    let temp_root = create_temp_plugin_dir("unsupported-keyword");
    fs::write(
        temp_root.join("plugin.json"),
        r#"{
            "plugin_id": "vendor.example.static",
            "spec_version": "1.0",
            "name": "Unsupported Keyword",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json"
        }"#,
    )
    .expect("写入 plugin.json 失败");
    fs::write(
        temp_root.join("schema.json"),
        r#"{
            "type": "object",
            "oneOf": [],
            "required": ["url"],
            "properties": {
                "url": { "type": "string" }
            }
        }"#,
    )
    .expect("写入 schema.json 失败");

    let loader = PluginLoader::new();
    let error = loader
        .load_from_dir(&temp_root)
        .expect_err("含 oneOf 时应失败");

    assert!(matches!(error, PluginRuntimeError::Invalid(_)));
    cleanup_dir(&temp_root);
}

fn builtins_static_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
}

fn create_temp_plugin_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间异常")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("subforge-plugin-runtime-{prefix}-{nanos}"));
    fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}

fn cleanup_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
