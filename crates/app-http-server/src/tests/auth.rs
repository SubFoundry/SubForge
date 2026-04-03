use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header::HOST};
use tower::ServiceExt;

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
