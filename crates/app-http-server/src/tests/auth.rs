use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header::HOST};
use std::io::Write as _;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

use super::*;

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
async fn health_endpoint_rejects_invalid_host_header() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(HOST, "evil.com")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = read_json(response).await;
    assert_eq!(
        body.get("code").and_then(serde_json::Value::as_str),
        Some("E_AUTH")
    );
}

#[tokio::test]
async fn health_response_does_not_include_cors_allow_origin_header() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
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
async fn events_endpoint_requires_admin_token() {
    let app = build_router(build_test_state());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let authorized_response = app
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(authorized_response.status(), StatusCode::OK);
    assert!(
        authorized_response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );
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
async fn shutdown_endpoint_requires_admin_token() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/system/shutdown")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn shutdown_endpoint_accepts_admin_header() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/system/shutdown")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_token_rotate_requires_admin_token() {
    let app = build_router(build_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin-token/rotate")
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_token_rotate_replaces_in_memory_and_file_token() {
    let state = build_test_state();
    let app = build_router(state.clone());

    let rotate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin-token/rotate")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(rotate_response.status(), StatusCode::OK);
    let rotate_payload = read_json(rotate_response).await;
    let new_token = rotate_payload
        .get("token")
        .and_then(serde_json::Value::as_str)
        .expect("轮换响应缺少 token")
        .to_string();
    assert_ne!(new_token, "test-admin-token");
    assert_eq!(new_token.len(), 43);

    let old_token_response = app
        .clone()
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
    assert_eq!(old_token_response.status(), StatusCode::UNAUTHORIZED);

    let new_token_response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", format!("Bearer {new_token}"))
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(new_token_response.status(), StatusCode::OK);

    let persisted = std::fs::read_to_string(state.admin_token_path.as_path())
        .expect("读取 admin_token 文件失败");
    assert_eq!(persisted.trim(), new_token);
}

#[tokio::test]
async fn auth_failures_trigger_rate_limit_after_five_invalid_attempts() {
    let app = build_router(build_test_state());

    for _ in 0..4 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/plugins")
                    .header(HOST, "127.0.0.1:18118")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .expect("创建请求失败"),
            )
            .await
            .expect("请求执行失败");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let threshold_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(threshold_response.status(), StatusCode::TOO_MANY_REQUESTS);
    let threshold_payload = read_json(threshold_response).await;
    assert_eq!(
        threshold_payload
            .get("code")
            .and_then(serde_json::Value::as_str),
        Some("E_RATE_LIMIT")
    );

    let cooldown_response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(cooldown_response.status(), StatusCode::TOO_MANY_REQUESTS);
    let cooldown_payload = read_json(cooldown_response).await;
    assert_eq!(
        cooldown_payload
            .get("code")
            .and_then(serde_json::Value::as_str),
        Some("E_RATE_LIMIT")
    );
}

#[tokio::test]
async fn plugin_import_rejects_zip_path_traversal_entries() {
    let app = build_router(build_test_state());
    let boundary = "----subforge-path-traversal-boundary";
    let payload = build_path_traversal_zip_bytes();
    let request_body = build_multipart_plugin_body(boundary, &payload, "malicious.zip");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/import")
                .header(HOST, "127.0.0.1:18118")
                .header("authorization", "Bearer test-admin-token")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(request_body))
                .expect("创建请求失败"),
        )
        .await
        .expect("请求执行失败");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(
        payload.get("code").and_then(serde_json::Value::as_str),
        Some("E_CONFIG_INVALID")
    );
    assert!(
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("路径非法"))
    );

    let list_response = app
        .oneshot(admin_request(
            axum::http::Method::GET,
            "/api/plugins",
            Body::empty(),
        ))
        .await
        .expect("请求执行失败");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_payload = read_json(list_response).await;
    let plugins = list_payload
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .expect("响应应包含 plugins 数组");
    assert!(plugins.is_empty(), "非法插件包不应被安装");
}

fn build_path_traversal_zip_bytes() -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        writer
            .start_file("../escape.txt", options)
            .expect("写入 zip 条目失败");
        writer
            .write_all(b"malicious payload")
            .expect("写入 zip 内容失败");
        writer.finish().expect("完成 zip 构建失败");
    }
    cursor.into_inner()
}
