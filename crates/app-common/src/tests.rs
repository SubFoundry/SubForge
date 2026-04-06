use std::collections::BTreeMap;

use crate::{
    AppError, AppSetting, ConfigSchema, Plugin, PluginManifest, PluginType, Profile, ProfileSource,
    ProxyNode, ProxyProtocol, ProxyTransport, RoutingTemplateGroupIr, RoutingTemplateIr,
    RoutingTemplateSourceKernel, SourceInstance, TlsConfig,
};

#[test]
fn app_error_fields_are_serializable() {
    let err = AppError::new("E_TEST", "测试错误", false);
    let json = serde_json::to_string(&err).expect("序列化失败");
    assert!(json.contains("\"code\":\"E_TEST\""));
    assert!(json.contains("\"message\":\"测试错误\""));
    assert!(json.contains("\"retryable\":false"));
}

#[test]
fn domain_models_are_serializable() {
    let plugin = Plugin {
        id: "plugin-row-1".to_string(),
        plugin_id: "vendor.example.static".to_string(),
        name: "Example Plugin".to_string(),
        version: "1.0.0".to_string(),
        spec_version: "1.0".to_string(),
        plugin_type: "static".to_string(),
        status: "enabled".to_string(),
        installed_at: "2026-04-02T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
    };
    let source = SourceInstance {
        id: "source-1".to_string(),
        plugin_id: plugin.plugin_id.clone(),
        name: "Source A".to_string(),
        status: "healthy".to_string(),
        state_json: Some("{\"cursor\":1}".to_string()),
        created_at: "2026-04-02T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
    };
    let profile = Profile {
        id: "profile-1".to_string(),
        name: "Default".to_string(),
        description: Some("默认聚合配置".to_string()),
        routing_template_source_id: Some("source-1".to_string()),
        created_at: "2026-04-02T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
    };
    let profile_source = ProfileSource {
        profile_id: profile.id.clone(),
        source_instance_id: source.id.clone(),
        priority: 10,
    };
    let setting = AppSetting {
        key: "ui.theme".to_string(),
        value: "dark".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
    };
    let node = ProxyNode {
        id: "node-1".to_string(),
        name: "HK-01".to_string(),
        protocol: ProxyProtocol::Vmess,
        server: "hk.example.com".to_string(),
        port: 443,
        transport: ProxyTransport::Ws,
        tls: TlsConfig {
            enabled: true,
            server_name: Some("hk.example.com".to_string()),
        },
        extra: BTreeMap::new(),
        source_id: "source-1".to_string(),
        tags: vec!["hk".to_string()],
        region: Some("hk".to_string()),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
    };

    assert!(
        serde_json::from_str::<Plugin>(&serde_json::to_string(&plugin).expect("plugin 序列化失败"))
            .is_ok()
    );
    assert!(
        serde_json::from_str::<SourceInstance>(
            &serde_json::to_string(&source).expect("source 序列化失败")
        )
        .is_ok()
    );
    assert!(
        serde_json::from_str::<Profile>(
            &serde_json::to_string(&profile).expect("profile 序列化失败")
        )
        .is_ok()
    );
    assert!(
        serde_json::from_str::<ProfileSource>(
            &serde_json::to_string(&profile_source).expect("profile_source 序列化失败")
        )
        .is_ok()
    );
    assert!(
        serde_json::from_str::<AppSetting>(
            &serde_json::to_string(&setting).expect("setting 序列化失败")
        )
        .is_ok()
    );
    assert!(
        serde_json::from_str::<ProxyNode>(
            &serde_json::to_string(&node).expect("proxy_node 序列化失败")
        )
        .is_ok()
    );
}

#[test]
fn plugin_manifest_is_deserializable() {
    let raw = serde_json::json!({
        "plugin_id": "vendor.example.static",
        "spec_version": "1.0",
        "name": "Static Source",
        "version": "1.0.0",
        "type": "static",
        "config_schema": "schema.json",
        "secret_fields": [],
        "capabilities": ["http", "json"],
        "network_profile": "standard",
        "anti_bot_level": "low"
    });

    let manifest: PluginManifest = serde_json::from_value(raw).expect("manifest 反序列化失败");
    assert_eq!(manifest.plugin_type, PluginType::Static);
    assert_eq!(manifest.config_schema, "schema.json");
    assert_eq!(manifest.network_profile, "standard");
}

#[test]
fn config_schema_is_deserializable() {
    let raw = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["url"],
        "properties": {
            "url": {
                "type": "string",
                "title": "订阅地址",
                "minLength": 1
            }
        },
        "additionalProperties": false
    });

    let schema: ConfigSchema = serde_json::from_value(raw).expect("schema 反序列化失败");
    assert_eq!(schema.schema_type, "object");
    assert!(schema.properties.contains_key("url"));
    assert_eq!(schema.required, vec!["url".to_string()]);
}

#[test]
fn routing_template_ir_can_convert_to_clash_template() {
    let ir = RoutingTemplateIr {
        groups: vec![RoutingTemplateGroupIr {
            name: "Proxy".to_string(),
            group_type: "select".to_string(),
            proxies: vec!["Auto".to_string(), "DIRECT".to_string()],
            url: None,
            interval: None,
            tolerance: None,
            include_all: false,
            use_provider: false,
            filter: None,
            exclude_filter: None,
        }],
        rules: vec!["MATCH,Proxy".to_string()],
        source_kernel: RoutingTemplateSourceKernel::SingBox,
        meta: None,
    };

    let template = ir.into_clash_template();
    assert_eq!(template.groups.len(), 1);
    assert_eq!(template.groups[0].name, "Proxy");
    assert_eq!(template.rules, vec!["MATCH,Proxy".to_string()]);
}
