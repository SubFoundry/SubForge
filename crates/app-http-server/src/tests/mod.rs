use std::fs;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use app_secrets::MemorySecretStore;
use app_storage::{ExportTokenRepository, RefreshJob, RefreshJobRepository};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{CONTENT_TYPE, HOST};
use axum::http::{HeaderMap, HeaderValue, Method, Request};
use axum::routing::get;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use zip::write::SimpleFileOptions;

use super::{ApiEvent, ServerContext, build_router};

mod auth;
mod e2e;
mod helpers_sanitization;

pub(super) fn build_test_state() -> ServerContext {
    let database = Arc::new(app_storage::Database::open_in_memory().expect("初始化数据库失败"));
    let secret_store = Arc::new(MemorySecretStore::new());
    let data_dir = std::env::temp_dir().join(format!(
        "subforge-http-server-test-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let plugins_dir = data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("创建测试插件目录失败");
    let admin_token_path = data_dir.join("admin_token");
    fs::write(&admin_token_path, "test-admin-token\n").expect("初始化测试 admin_token 文件失败");
    let (tx, _rx) = tokio::sync::broadcast::channel::<ApiEvent>(64);
    ServerContext::new(
        "test-admin-token".to_string(),
        admin_token_path,
        database,
        secret_store,
        plugins_dir,
        18118,
        tx,
    )
}

pub(super) fn admin_request(method: Method, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(HOST, "127.0.0.1:18118")
        .header("authorization", "Bearer test-admin-token")
        .body(body)
        .expect("构建管理请求失败")
}

pub(super) fn admin_json_request(method: Method, uri: &str, payload: &Value) -> Request<Body> {
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

pub(super) async fn read_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体失败");
    serde_json::from_slice::<Value>(&body).expect("响应体不是合法 JSON")
}

pub(super) async fn wait_refresh_complete_event(
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

pub(super) fn build_builtin_plugin_zip_bytes() -> Vec<u8> {
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

pub(super) fn build_builtin_plugin_zip_bytes_with_root_dir(root_dir: &str) -> Vec<u8> {
    let plugin_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static");
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for file_name in ["plugin.json", "schema.json"] {
            let zip_entry = format!("{root_dir}/{file_name}");
            writer
                .start_file(zip_entry, options)
                .expect("写入 zip 条目失败");
            let bytes = fs::read(plugin_dir.join(file_name)).expect("读取内置插件文件失败");
            writer.write_all(&bytes).expect("写入 zip 数据失败");
        }
        writer.finish().expect("完成 zip 构建失败");
    }
    cursor.into_inner()
}

pub(super) fn script_mock_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/script-mock")
}

pub(super) fn build_script_mock_plugin_zip_bytes() -> Vec<u8> {
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

pub(super) fn build_multipart_plugin_body(
    boundary: &str,
    zip_payload: &[u8],
    filename: &str,
) -> Vec<u8> {
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

pub(super) async fn start_fixture_server(
    body: String,
    content_type: &'static str,
) -> (String, JoinHandle<()>) {
    start_fixture_server_with_userinfo(body, content_type, None).await
}

pub(super) async fn start_fixture_server_with_userinfo(
    body: String,
    content_type: &'static str,
    subscription_userinfo: Option<&'static str>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/sub",
        get(move || {
            let body = body.clone();
            let subscription_userinfo = subscription_userinfo;
            async move {
                let mut headers = HeaderMap::new();
                headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
                if let Some(value) = subscription_userinfo {
                    headers.insert("subscription-userinfo", HeaderValue::from_static(value));
                }
                (headers, body)
            }
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

pub(super) const BASE64_SUBSCRIPTION_FIXTURE: &str =
    include_str!("../../../app-core/tests/fixtures/subscription_base64.txt");
