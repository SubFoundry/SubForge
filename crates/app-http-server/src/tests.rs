use std::fs;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use app_secrets::MemorySecretStore;
use app_storage::{ExportTokenRepository, RefreshJobRepository};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header::CONTENT_TYPE, header::HOST};
use axum::routing::get;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

use super::{ApiEvent, ServerContext, build_router};
fn build_test_state() -> ServerContext {
    let database = Arc::new(app_storage::Database::open_in_memory().expect("初始化数据库失败"));
    let secret_store = Arc::new(MemorySecretStore::new());
    let plugins_dir = std::env::temp_dir().join(format!(
        "subforge-http-server-test-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(&plugins_dir).expect("创建测试插件目录失败");
    let (tx, _rx) = tokio::sync::broadcast::channel::<ApiEvent>(64);
    ServerContext::new(
        "test-admin-token".to_string(),
        database,
        secret_store,
        plugins_dir,
        18118,
        tx,
    )
}

#[tokio::test]
async fn plugins_api_requires_admin_token() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn plugins_api_rejects_query_admin_token() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugins?token=test-admin-token")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn plugins_api_accepts_admin_header() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 64)
        .await
        .expect("读取响应体失败");
    let raw = String::from_utf8(body.to_vec()).expect("响应体不是 UTF-8");
    assert!(raw.contains("\"plugins\""));
}

#[tokio::test]
async fn options_preflight_returns_204_without_cors_header() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[tokio::test]
async fn e2e_import_source_refresh_and_raw_profile_output() {
    let state = build_test_state();
    let mut event_receiver = state.event_sender.subscribe();
    let app = build_router(state.clone());

    let (upstream_base, server_task) = start_fixture_server(
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let boundary = "----subforge-e2e-boundary";
    let plugin_zip = build_builtin_plugin_zip_bytes();
    let import_body = build_multipart_plugin_body(boundary, &plugin_zip, "builtin-static.zip");
    let import_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/plugins/import")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(import_body))
                .expect("构建导入插件请求失败"),
        )
        .await
        .expect("导入插件请求执行失败");
    assert_eq!(import_response.status(), StatusCode::CREATED);

    let source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "E2E Source",
                "config": {
                    "url": format!("{upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建来源请求执行失败");
    assert_eq!(source_response.status(), StatusCode::CREATED);
    let source_payload = read_json(source_response).await;
    let source_id = source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("来源响应缺少 source.id")
        .to_string();

    let profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/profiles",
            &json!({
                "name": "E2E Profile",
                "source_ids": [source_id.clone()]
            }),
        ))
        .await
        .expect("创建 Profile 请求执行失败");
    assert_eq!(profile_response.status(), StatusCode::CREATED);
    let profile_payload = read_json(profile_response).await;
    let profile_id = profile_payload
        .pointer("/profile/profile/id")
        .and_then(Value::as_str)
        .expect("Profile 响应缺少 id")
        .to_string();

    let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
    let export_token = export_token_repository
        .get_active_token(&profile_id)
        .expect("读取 export_token 失败")
        .expect("创建 Profile 后应自动生成 export_token")
        .token;

    let refresh_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/sources/{source_id}/refresh"),
            Body::empty(),
        ))
        .await
        .expect("刷新来源请求执行失败");
    assert_eq!(refresh_response.status(), StatusCode::OK);
    let refresh_payload = read_json(refresh_response).await;
    assert_eq!(
        refresh_payload.get("source_id").and_then(Value::as_str),
        Some(source_id.as_str())
    );
    assert_eq!(
        refresh_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );

    let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
    let refresh_jobs = refresh_repository
        .list_by_source(&source_id)
        .expect("读取 refresh_jobs 失败");
    assert_eq!(refresh_jobs.len(), 1);
    assert_eq!(refresh_jobs[0].status, "success");
    assert_eq!(refresh_jobs[0].node_count, Some(3));

    let event = wait_refresh_complete_event(&mut event_receiver, &source_id).await;
    assert_eq!(event.event, "refresh:complete");
    assert_eq!(event.source_id.as_deref(), Some(source_id.as_str()));

    let raw_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/raw?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建 raw 请求失败"),
        )
        .await
        .expect("读取 raw 订阅请求执行失败");
    assert_eq!(raw_response.status(), StatusCode::OK);
    let raw_payload = read_json(raw_response).await;
    assert_eq!(
        raw_payload.get("profile_id").and_then(Value::as_str),
        Some(profile_id.as_str())
    );
    assert_eq!(
        raw_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        raw_payload
            .get("nodes")
            .and_then(Value::as_array)
            .map(|items| items.len()),
        Some(3)
    );

    server_task.abort();
}

#[tokio::test]
async fn e2e_script_source_refresh_via_management_api() {
    let state = build_test_state();
    let mut event_receiver = state.event_sender.subscribe();
    let app = build_router(state.clone());

    let (upstream_base, server_task) = start_fixture_server(
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let boundary = "----subforge-e2e-script-boundary";
    let plugin_zip = build_script_mock_plugin_zip_bytes();
    let import_body = build_multipart_plugin_body(boundary, &plugin_zip, "script-mock.zip");
    let import_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/plugins/import")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(import_body))
                .expect("构建脚本插件导入请求失败"),
        )
        .await
        .expect("导入脚本插件请求执行失败");
    assert_eq!(import_response.status(), StatusCode::CREATED);

    let source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "vendor.example.script-mock",
                "name": "Script E2E Source",
                "config": {
                    "subscription_url": format!("{upstream_base}/sub"),
                    "username": "alice",
                    "password": "wonderland"
                }
            }),
        ))
        .await
        .expect("创建脚本来源请求执行失败");
    assert_eq!(source_response.status(), StatusCode::CREATED);
    let source_payload = read_json(source_response).await;
    let source_id = source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("脚本来源响应缺少 source.id")
        .to_string();

    let profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/profiles",
            &json!({
                "name": "Script E2E Profile",
                "source_ids": [source_id.clone()]
            }),
        ))
        .await
        .expect("创建脚本 Profile 请求执行失败");
    assert_eq!(profile_response.status(), StatusCode::CREATED);
    let profile_payload = read_json(profile_response).await;
    let profile_id = profile_payload
        .pointer("/profile/profile/id")
        .and_then(Value::as_str)
        .expect("脚本 Profile 响应缺少 id")
        .to_string();

    let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
    let export_token = export_token_repository
        .get_active_token(&profile_id)
        .expect("读取脚本 Profile export_token 失败")
        .expect("创建脚本 Profile 后应自动生成 export_token")
        .token;

    let refresh_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/sources/{source_id}/refresh"),
            Body::empty(),
        ))
        .await
        .expect("刷新脚本来源请求执行失败");
    let refresh_status = refresh_response.status();
    let refresh_payload = read_json(refresh_response).await;
    assert_eq!(
        refresh_status,
        StatusCode::OK,
        "脚本来源刷新应成功，实际返回：{refresh_payload:?}"
    );
    assert_eq!(
        refresh_payload.get("source_id").and_then(Value::as_str),
        Some(source_id.as_str())
    );
    assert_eq!(
        refresh_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );

    let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
    let refresh_jobs = refresh_repository
        .list_by_source(&source_id)
        .expect("读取脚本 refresh_jobs 失败");
    assert_eq!(refresh_jobs.len(), 1);
    assert_eq!(refresh_jobs[0].status, "success");
    assert_eq!(refresh_jobs[0].node_count, Some(3));

    let event = wait_refresh_complete_event(&mut event_receiver, &source_id).await;
    assert_eq!(event.event, "refresh:complete");
    assert_eq!(event.source_id.as_deref(), Some(source_id.as_str()));

    let raw_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/raw?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建脚本 raw 请求失败"),
        )
        .await
        .expect("读取脚本 raw 订阅请求执行失败");
    assert_eq!(raw_response.status(), StatusCode::OK);
    let raw_payload = read_json(raw_response).await;
    assert_eq!(
        raw_payload.get("profile_id").and_then(Value::as_str),
        Some(profile_id.as_str())
    );
    assert_eq!(
        raw_payload.get("node_count").and_then(Value::as_u64),
        Some(3)
    );

    let source_repository = app_storage::SourceRepository::new(state.database.as_ref());
    let persisted_state_raw = source_repository
        .get_by_id(&source_id)
        .expect("读取脚本来源失败")
        .and_then(|source| source.state_json)
        .expect("脚本来源刷新后应写入 state_json");
    let persisted_state: Value =
        serde_json::from_str(&persisted_state_raw).expect("state_json 必须是合法 JSON");
    assert_eq!(
        persisted_state.get("counter").and_then(Value::as_u64),
        Some(3)
    );

    server_task.abort();
}

fn admin_request(method: Method, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(HOST, "127.0.0.1:18118")
        .header("authorization", "Bearer test-admin-token")
        .body(body)
        .expect("构建管理请求失败")
}

fn admin_json_request(method: Method, uri: &str, payload: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(HOST, "127.0.0.1:18118")
        .header("authorization", "Bearer test-admin-token")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(payload).expect("序列化 JSON 请求体失败"),
        ))
        .expect("构建管理 JSON 请求失败")
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体失败");
    serde_json::from_slice::<Value>(&body).expect("响应体不是合法 JSON")
}

async fn wait_refresh_complete_event(
    receiver: &mut tokio::sync::broadcast::Receiver<ApiEvent>,
    source_id: &str,
) -> ApiEvent {
    timeout(Duration::from_secs(5), async {
        loop {
            let event = receiver.recv().await.expect("读取事件失败");
            if event.event == "refresh:complete" && event.source_id.as_deref() == Some(source_id) {
                return event;
            }
        }
    })
    .await
    .expect("等待 refresh:complete 事件超时")
}

fn build_builtin_plugin_zip_bytes() -> Vec<u8> {
    let plugin_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static");
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for file_name in ["plugin.json", "schema.json"] {
            writer
                .start_file(file_name, options)
                .expect("写入 zip 条目失败");
            let bytes = fs::read(plugin_dir.join(file_name)).expect("读取内置插件文件失败");
            writer.write_all(&bytes).expect("写入 zip 数据失败");
        }
        writer.finish().expect("完成 zip 构建失败");
    }
    cursor.into_inner()
}

fn script_mock_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/script-mock")
}

fn build_script_mock_plugin_zip_bytes() -> Vec<u8> {
    let plugin_dir = script_mock_plugin_dir();
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for file_name in [
            "plugin.json",
            "schema.json",
            "scripts/login.lua",
            "scripts/refresh.lua",
            "scripts/fetch.lua",
        ] {
            writer
                .start_file(file_name, options)
                .expect("写入脚本插件 zip 条目失败");
            let bytes = fs::read(plugin_dir.join(file_name)).expect("读取脚本插件文件失败");
            writer.write_all(&bytes).expect("写入脚本插件 zip 数据失败");
        }
        writer.finish().expect("完成脚本插件 zip 构建失败");
    }
    cursor.into_inner()
}

fn build_multipart_plugin_body(boundary: &str, zip_payload: &[u8], filename: &str) -> Vec<u8> {
    let mut body = Vec::new();
    write!(body, "--{boundary}\r\n").expect("写入 multipart 边界失败");
    write!(
        body,
        "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
    )
    .expect("写入 multipart disposition 失败");
    write!(body, "Content-Type: application/zip\r\n\r\n")
        .expect("写入 multipart content-type 失败");
    body.extend_from_slice(zip_payload);
    write!(body, "\r\n--{boundary}--\r\n").expect("写入 multipart 结束边界失败");
    body
}

async fn start_fixture_server(
    body: String,
    content_type: &'static str,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/sub",
        get(move || {
            let body = body.clone();
            async move { ([(axum::http::header::CONTENT_TYPE, content_type)], body) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试上游服务器失败");
    let address: SocketAddr = listener.local_addr().expect("读取测试监听地址失败");
    let base_url = format!("http://{address}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试上游服务器运行失败");
    });
    (base_url, handle)
}

const BASE64_SUBSCRIPTION_FIXTURE: &str =
    include_str!("../../app-core/tests/fixtures/subscription_base64.txt");
