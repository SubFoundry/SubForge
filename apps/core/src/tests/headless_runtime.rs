use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use app_http_server::{ApiEvent, ServerContext, build_router as build_http_router};
use app_secrets::{MemorySecretStore, SecretStore};
use app_storage::{Database, ExportTokenRepository, ProfileRepository};
use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::config::LoadedHeadlessConfig;
use crate::headless::{
    apply_headless_configuration, apply_headless_settings, list_profile_source_ids, list_sources,
};

const BASE64_SUBSCRIPTION_FIXTURE: &str = "c3M6Ly9ZV1Z6TFRJMU5pMW5ZMjA2Y0dGemMzZHZjbVE9QGhrLmV4YW1wbGUuY29tOjQ0MyNISy1TUwp2bWVzczovL2V5SjJJam9pTWlJc0luQnpJam9pVTBjdFZrMUZVMU1pTENKaFpHUWlPaUp6Wnk1bGVHRnRjR3hsTG1OdmJTSXNJbkJ2Y25RaU9pSTBORE1pTENKcFpDSTZJakV4TVRFeE1URXhMVEV4TVRFdE1URXhNUzB4TVRFeExURXhNVEV4TVRFeE1URXhNU0lzSW1GcFpDSTZJakFpTENKdVpYUWlPaUozY3lJc0luUnNjeUk2SW5Sc2N5SXNJbkJoZEdnaU9pSXZkM01pTENKb2IzTjBJam9pYzJjdVpYaGhiWEJzWlM1amIyMGlmUT09Cm5vdC1hLXVyaS1saW5lCnRyb2phbjovL3Bhc3N3b3JkMTIzQHVzLmV4YW1wbGUuY29tOjQ0Mz9zbmk9dXMuZXhhbXBsZS5jb20jVVMtVHJvamFu";

#[tokio::test]
async fn headless_config_can_build_model_and_export_after_refresh() {
    let temp_root = create_temp_dir("headless-run-c-e2e");
    let config_path = temp_root.join("subforge.toml");
    let runtime_plugins_dir = temp_root.join("runtime-plugins");
    std::fs::create_dir_all(&runtime_plugins_dir).expect("创建运行时插件目录失败");

    let (upstream_base, upstream_shutdown) = spawn_fixture_upstream().await;
    std::fs::write(
        &config_path,
        format!(
            r#"
[server]
listen = "127.0.0.1:19131"

[plugins]
dirs = ["{plugin_dir}"]

[[sources]]
name = "headless-static"
plugin = "subforge.builtin.static"
[sources.config]
url = "{upstream_base}/sub"

[[profiles]]
name = "headless-profile"
sources = ["headless-static"]
export_token = "headless-token"
"#,
            plugin_dir = path_to_toml_string(&builtin_plugin_dir()),
            upstream_base = upstream_base
        ),
    )
    .expect("写入配置文件失败");

    let loaded = LoadedHeadlessConfig::from_file(&config_path).expect("加载配置失败");
    let database = Arc::new(Database::open_in_memory().expect("初始化内存数据库失败"));
    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());

    apply_headless_settings(&loaded, database.as_ref()).expect("应用无头设置失败");
    let report = apply_headless_configuration(
        &loaded,
        database.as_ref(),
        Arc::clone(&secret_store),
        &runtime_plugins_dir,
    )
    .expect("应用无头配置失败");
    assert_eq!(report.created_sources, 1);
    assert_eq!(report.created_profiles, 1);

    let sources = list_sources(database.as_ref()).expect("读取来源失败");
    assert_eq!(sources.len(), 1, "应由配置自动创建来源");
    let source_id = sources[0].id.clone();

    let profiles = ProfileRepository::new(database.as_ref())
        .list()
        .expect("读取 Profile 失败");
    assert_eq!(profiles.len(), 1, "应由配置自动创建 Profile");
    let profile_id = profiles[0].id.clone();
    let profile_source_ids =
        list_profile_source_ids(database.as_ref(), &profile_id).expect("读取 Profile 来源关联失败");
    assert_eq!(profile_source_ids, vec![source_id.clone()]);

    let export_token = ExportTokenRepository::new(database.as_ref())
        .get_active_token(&profile_id)
        .expect("读取 export token 失败")
        .expect("配置应用后应存在 export token")
        .token;
    assert_eq!(export_token, "headless-token");

    let admin_token = "headless-admin-token".to_string();
    let (api_base, api_shutdown) = spawn_api_server(
        Arc::clone(&database),
        Arc::clone(&secret_store),
        runtime_plugins_dir.clone(),
        admin_token.clone(),
    )
    .await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("构建 HTTP 客户端失败");

    let refresh_response = client
        .post(format!("{api_base}/api/sources/{source_id}/refresh"))
        .bearer_auth(&admin_token)
        .send()
        .await
        .expect("触发来源刷新失败");
    assert_eq!(refresh_response.status(), reqwest::StatusCode::OK);
    let refresh_payload: Value = refresh_response
        .json()
        .await
        .expect("解析刷新响应 JSON 失败");
    assert_eq!(
        refresh_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );

    let raw_response = client
        .get(format!(
            "{api_base}/api/profiles/{profile_id}/raw?token={export_token}"
        ))
        .send()
        .await
        .expect("读取 raw 订阅失败");
    assert_eq!(raw_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        raw_response
            .headers()
            .get("profile-title")
            .and_then(|value| value.to_str().ok()),
        Some("headless-profile")
    );
    let raw_payload: Value = raw_response.json().await.expect("解析 raw 响应 JSON 失败");
    assert_eq!(
        raw_payload.get("profile_id").and_then(Value::as_str),
        Some(profile_id.as_str())
    );
    assert_eq!(
        raw_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );

    let clash_response = client
        .get(format!(
            "{api_base}/api/profiles/{profile_id}/clash?token={export_token}"
        ))
        .send()
        .await
        .expect("读取 clash 订阅失败");
    assert_eq!(clash_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        clash_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/yaml; charset=utf-8")
    );
    let clash_text = clash_response.text().await.expect("读取 clash 文本失败");
    assert!(clash_text.contains("proxies:"));
    assert!(clash_text.contains("proxy-groups:"));

    let singbox_response = client
        .get(format!(
            "{api_base}/api/profiles/{profile_id}/sing-box?token={export_token}"
        ))
        .send()
        .await
        .expect("读取 sing-box 订阅失败");
    assert_eq!(singbox_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        singbox_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json; charset=utf-8")
    );
    let singbox_json: Value = singbox_response
        .json()
        .await
        .expect("解析 sing-box 响应 JSON 失败");
    assert!(singbox_json.get("outbounds").is_some());

    let base64_response = client
        .get(format!(
            "{api_base}/api/profiles/{profile_id}/base64?token={export_token}"
        ))
        .send()
        .await
        .expect("读取 base64 订阅失败");
    assert_eq!(base64_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        base64_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let encoded = base64_response.text().await.expect("读取 base64 文本失败");
    let decoded = BASE64_STANDARD
        .decode(encoded.as_bytes())
        .expect("base64 响应应可解码");
    let decoded_text = String::from_utf8(decoded).expect("base64 解码内容应为 UTF-8");
    assert!(
        decoded_text.lines().any(|line| line.starts_with("ss://")),
        "解码内容中应包含 ss:// 链接"
    );

    let _ = api_shutdown.send(());
    let _ = upstream_shutdown.send(());
    let _ = std::fs::remove_dir_all(&temp_root);
}

async fn spawn_fixture_upstream() -> (String, oneshot::Sender<()>) {
    let app = Router::new().route(
        "/sub",
        get(|| async {
            (
                [(CONTENT_TYPE, "text/plain; charset=utf-8")],
                BASE64_SUBSCRIPTION_FIXTURE,
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动上游 fixture 服务失败");
    let addr = listener.local_addr().expect("读取上游监听地址失败");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let server = axum::serve(listener, app);
        let graceful = server.with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = graceful.await {
            panic!("上游 fixture 服务异常退出: {error}");
        }
    });
    (format!("http://{addr}"), shutdown_tx)
}

async fn spawn_api_server(
    database: Arc<Database>,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: PathBuf,
    admin_token: String,
) -> (String, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 API 服务失败");
    let addr = listener.local_addr().expect("读取 API 监听地址失败");
    let port = addr.port();
    let (event_sender, _event_receiver) = tokio::sync::broadcast::channel::<ApiEvent>(64);
    let app = build_http_router(ServerContext::new(
        admin_token,
        database,
        secret_store,
        plugins_dir,
        port,
        event_sender,
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let server = axum::serve(listener, app);
        let graceful = server.with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = graceful.await {
            panic!("API 服务异常退出: {error}");
        }
    });
    (format!("http://{addr}"), shutdown_tx)
}

fn builtin_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
}

fn path_to_toml_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "subforge-{prefix}-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}
