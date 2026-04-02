use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use app_common::ProxyProtocol;
use app_secrets::{MemorySecretStore, SecretStore};
use app_storage::{
    Database, ExportTokenRepository, NodeCacheRepository, PluginRepository, RefreshJobRepository,
    SourceConfigRepository, SourceRepository,
};
use axum::Router;
use axum::http::{HeaderMap as AxumHeaderMap, StatusCode};
use axum::routing::get;
use reqwest::Url;
use reqwest::header::{ACCEPT, AUTHORIZATION, COOKIE, HeaderMap as ReqwestHeaderMap, HeaderValue};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::fetcher::{redact_headers_for_log, redact_url_for_log};
use crate::{
    CoreError, Engine, PluginInstallService, SourceService, StaticFetcher, SubscriptionParser,
    UriListParser,
};

const BASE64_SUBSCRIPTION_FIXTURE: &str =
    include_str!("../../tests/fixtures/subscription_base64.txt");

mod engine;
mod fetcher;
mod logging_redaction;
mod parser;
mod plugin_install;
mod source_service;

fn builtins_static_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间异常")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("subforge-app-core-{prefix}-{nanos}"));
    fs::create_dir_all(&path).expect("创建临时目录失败");
    path
}

fn create_bad_plugin_dir(base: &Path) -> PathBuf {
    let path = base.join("invalid-plugin");
    fs::create_dir_all(&path).expect("创建非法插件目录失败");
    fs::write(
        path.join("plugin.json"),
        r#"{
            "plugin_id": "vendor.example.invalid",
            "spec_version": "1.0",
            "name": "Invalid Plugin",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json"
        }"#,
    )
    .expect("写入非法插件 plugin.json 失败");
    fs::write(path.join("schema.json"), r#"{"type":"object","oneOf":[]}"#)
        .expect("写入非法插件 schema.json 失败");
    path
}

fn create_upgraded_plugin_dir(base: &Path) -> PathBuf {
    let path = base.join("upgraded-plugin");
    fs::create_dir_all(&path).expect("创建升级插件目录失败");
    fs::copy(
        builtins_static_plugin_dir().join("schema.json"),
        path.join("schema.json"),
    )
    .expect("复制 schema.json 失败");
    let plugin_json = fs::read_to_string(builtins_static_plugin_dir().join("plugin.json"))
        .expect("读取内置 plugin.json 失败")
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.0.1\"");
    fs::write(path.join("plugin.json"), plugin_json).expect("写入升级插件 plugin.json 失败");
    path
}

fn create_secret_static_plugin_dir(base: &Path) -> PathBuf {
    let path = base.join("secure-static-plugin");
    fs::create_dir_all(&path).expect("创建插件目录失败");
    fs::write(
        path.join("plugin.json"),
        r#"{
            "plugin_id": "vendor.example.secure-static",
            "spec_version": "1.0",
            "name": "Secure Static Source",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json",
            "secret_fields": ["token"],
            "capabilities": ["http", "json"],
            "network_profile": "standard"
        }"#,
    )
    .expect("写入 plugin.json 失败");
    fs::write(
        path.join("schema.json"),
        r#"{
            "type": "object",
            "required": ["url", "token"],
            "properties": {
                "url": { "type": "string", "minLength": 1 },
                "token": { "type": "string", "minLength": 1, "format": "password" },
                "region": { "type": "string", "enum": ["auto", "hk", "sg", "us"], "default": "auto" }
            }
        }"#,
    )
    .expect("写入 schema.json 失败");
    path
}

fn create_static_plugin_with_network_profile(
    base: &Path,
    dir_name: &str,
    plugin_id: &str,
    network_profile: &str,
) -> PathBuf {
    let path = base.join(dir_name);
    fs::create_dir_all(&path).expect("创建插件目录失败");
    let plugin_json = format!(
        r#"{{
            "plugin_id": "{plugin_id}",
            "spec_version": "1.0",
            "name": "{plugin_id}",
            "version": "1.0.0",
            "type": "static",
            "config_schema": "schema.json",
            "capabilities": ["http", "json"],
            "network_profile": "{network_profile}"
        }}"#
    );
    fs::write(path.join("plugin.json"), plugin_json).expect("写入 plugin.json 失败");
    fs::write(
        path.join("schema.json"),
        r#"{
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": { "type": "string", "minLength": 1 }
            },
            "additionalProperties": false
        }"#,
    )
    .expect("写入 schema.json 失败");
    path
}

fn create_script_plugin_dir(
    base: &Path,
    dir_name: &str,
    plugin_id: &str,
    login_script: Option<&str>,
    refresh_script: Option<&str>,
    fetch_script: &str,
) -> PathBuf {
    let path = base.join(dir_name);
    let scripts_dir = path.join("scripts");
    fs::create_dir_all(&scripts_dir).expect("创建脚本插件目录失败");

    if let Some(login_script) = login_script {
        fs::write(scripts_dir.join("login.lua"), login_script).expect("写入 login.lua 失败");
    }
    if let Some(refresh_script) = refresh_script {
        fs::write(scripts_dir.join("refresh.lua"), refresh_script).expect("写入 refresh.lua 失败");
    }
    fs::write(scripts_dir.join("fetch.lua"), fetch_script).expect("写入 fetch.lua 失败");

    let login_entry = if login_script.is_some() {
        r#""login": "scripts/login.lua","#
    } else {
        ""
    };
    let refresh_entry = if refresh_script.is_some() {
        r#""refresh": "scripts/refresh.lua","#
    } else {
        ""
    };
    let plugin_json = format!(
        r#"{{
            "plugin_id": "{plugin_id}",
            "spec_version": "1.0",
            "name": "{plugin_id}",
            "version": "1.0.0",
            "type": "script",
            "config_schema": "schema.json",
            "secret_fields": [],
            "entrypoints": {{
                {login_entry}
                {refresh_entry}
                "fetch": "scripts/fetch.lua"
            }},
            "capabilities": ["http", "cookie", "json", "html", "base64", "secret", "log", "time"],
            "network_profile": "standard"
        }}"#
    );
    fs::write(path.join("plugin.json"), plugin_json).expect("写入脚本插件 plugin.json 失败");
    fs::write(
        path.join("schema.json"),
        r#"{
            "type": "object",
            "required": ["seed"],
            "properties": {
                "seed": { "type": "string", "minLength": 1 }
            },
            "additionalProperties": false
        }"#,
    )
    .expect("写入脚本插件 schema.json 失败");

    path
}

fn sample_source(id: &str, plugin_id: &str) -> app_common::SourceInstance {
    app_common::SourceInstance {
        id: id.to_string(),
        plugin_id: plugin_id.to_string(),
        name: format!("Source {id}"),
        status: "healthy".to_string(),
        state_json: None,
        created_at: "2026-04-02T01:10:00Z".to_string(),
        updated_at: "2026-04-02T01:10:00Z".to_string(),
    }
}

async fn start_fixture_server(
    route_path: &'static str,
    body: String,
    content_type: &'static str,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        route_path,
        get(move || {
            let body = body.clone();
            async move { ([(axum::http::header::CONTENT_TYPE, content_type)], body) }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试 HTTP 服务失败");
    let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
    let base_url = format!("http://{}", address);

    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试 HTTP 服务运行失败");
    });

    (base_url, task)
}

async fn start_profile_gate_server(
    route_path: &'static str,
    success_body: String,
    content_type: &'static str,
) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>, JoinHandle<()>) {
    let total_requests = Arc::new(AtomicUsize::new(0));
    let chrome_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        route_path,
        get({
            let total_requests = total_requests.clone();
            let chrome_requests = chrome_requests.clone();
            move |headers: AxumHeaderMap| {
                let success_body = success_body.clone();
                let total_requests = total_requests.clone();
                let chrome_requests = chrome_requests.clone();
                async move {
                    total_requests.fetch_add(1, Ordering::SeqCst);
                    let has_chrome_header = headers
                        .get("sec-ch-ua")
                        .and_then(|value| value.to_str().ok())
                        .map(|value| value.contains("Chromium"))
                        .unwrap_or(false);
                    let has_fetch_mode = headers
                        .get("sec-fetch-mode")
                        .and_then(|value| value.to_str().ok())
                        .map(|value| value == "navigate")
                        .unwrap_or(false);
                    if has_chrome_header && has_fetch_mode {
                        chrome_requests.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, content_type)],
                            success_body,
                        )
                    } else {
                        (
                            StatusCode::FORBIDDEN,
                            [(axum::http::header::CONTENT_TYPE, content_type)],
                            "missing browser_chrome fingerprint".to_string(),
                        )
                    }
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试 HTTP 服务失败");
    let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
    let base_url = format!("http://{}", address);

    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试 HTTP 服务运行失败");
    });

    (base_url, total_requests, chrome_requests, task)
}

async fn start_retry_fixture_server(
    route_path: &'static str,
    retry_statuses: Vec<u16>,
    success_body: String,
    content_type: &'static str,
) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
    let retry_statuses = Arc::new(retry_statuses);
    let request_count = Arc::new(AtomicUsize::new(0));

    let app = Router::new().route(
        route_path,
        get({
            let retry_statuses = retry_statuses.clone();
            let request_count = request_count.clone();
            move || {
                let retry_statuses = retry_statuses.clone();
                let request_count = request_count.clone();
                let success_body = success_body.clone();
                async move {
                    let current = request_count.fetch_add(1, Ordering::SeqCst);
                    if current < retry_statuses.len() {
                        let status =
                            StatusCode::from_u16(retry_statuses[current]).expect("状态码必须合法");
                        (
                            status,
                            [(axum::http::header::CONTENT_TYPE, content_type)],
                            "retry".to_string(),
                        )
                    } else {
                        (
                            StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, content_type)],
                            success_body,
                        )
                    }
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试 HTTP 服务失败");
    let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
    let base_url = format!("http://{}", address);

    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试 HTTP 服务运行失败");
    });

    (base_url, request_count, task)
}

fn cleanup_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
