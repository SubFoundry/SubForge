use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header::CONTENT_TYPE, header::HOST};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn e2e_import_source_refresh_and_raw_profile_output() {
    let state = build_test_state();
    let mut event_receiver = state.event_sender.subscribe();
    let app = build_router(state.clone());

    let (upstream_base, server_task) = start_fixture_server_with_userinfo(
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
        Some("upload=1; download=2; total=3; expire=1735689600"),
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

    let status_before_refresh = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            "/api/system/status",
            Body::empty(),
        ))
        .await
        .expect("读取系统状态失败");
    assert_eq!(status_before_refresh.status(), StatusCode::OK);
    let status_before_payload = read_json(status_before_refresh).await;
    assert_eq!(
        status_before_payload
            .get("active_sources")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        status_before_payload
            .get("total_nodes")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        status_before_payload
            .get("last_refresh_at")
            .and_then(Value::as_str),
        None
    );

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
    let failed_job = RefreshJob {
        id: "refresh-job-failed-manual".to_string(),
        source_instance_id: source_id.clone(),
        trigger_type: "manual".to_string(),
        status: "running".to_string(),
        started_at: Some("9999-01-01T00:00:00Z".to_string()),
        finished_at: None,
        node_count: None,
        error_code: None,
        error_message: None,
    };
    refresh_repository
        .insert(&failed_job)
        .expect("写入失败 refresh job 失败");
    refresh_repository
        .mark_failed(
            &failed_job.id,
            "9999-01-01T00:00:01Z",
            "E_HTTP_5XX",
            "upstream 502",
        )
        .expect("更新失败 refresh job 失败");

    let logs_response = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            "/api/logs?limit=5",
            Body::empty(),
        ))
        .await
        .expect("读取 logs 失败");
    assert_eq!(logs_response.status(), StatusCode::OK);
    let logs_payload = read_json(logs_response).await;
    let logs = logs_payload
        .get("logs")
        .and_then(Value::as_array)
        .expect("logs 响应缺少数组字段");
    assert!(!logs.is_empty());
    assert!(logs.iter().any(|entry| {
        entry.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
            && entry.get("status").and_then(Value::as_str) == Some("success")
    }));
    assert!(logs.iter().any(|entry| {
        entry.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
            && entry.get("status").and_then(Value::as_str) == Some("failed")
            && entry.get("error_code").and_then(Value::as_str) == Some("E_HTTP_5XX")
    }));

    let failed_logs_response = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            "/api/logs?status=failed&limit=5",
            Body::empty(),
        ))
        .await
        .expect("读取失败 logs 失败");
    assert_eq!(failed_logs_response.status(), StatusCode::OK);
    let failed_logs_payload = read_json(failed_logs_response).await;
    let failed_logs = failed_logs_payload
        .get("logs")
        .and_then(Value::as_array)
        .expect("failed logs 响应缺少数组字段");
    assert!(!failed_logs.is_empty());
    assert!(
        failed_logs
            .iter()
            .all(|entry| { entry.get("status").and_then(Value::as_str) == Some("failed") })
    );
    assert!(
        failed_logs.iter().any(|entry| {
            entry.get("source_name").and_then(Value::as_str) == Some("E2E Source")
        })
    );

    let event = wait_refresh_complete_event(&mut event_receiver, &source_id).await;
    assert_eq!(event.event, "refresh:complete");
    assert_eq!(event.source_id.as_deref(), Some(source_id.as_str()));

    let status_after_refresh = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            "/api/system/status",
            Body::empty(),
        ))
        .await
        .expect("刷新后读取系统状态失败");
    assert_eq!(status_after_refresh.status(), StatusCode::OK);
    let status_after_payload = read_json(status_after_refresh).await;
    assert_eq!(
        status_after_payload
            .get("active_sources")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        status_after_payload
            .get("total_nodes")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert!(
        status_after_payload
            .get("last_refresh_at")
            .and_then(Value::as_str)
            .is_some()
    );

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
    assert_eq!(
        raw_response
            .headers()
            .get("profile-title")
            .and_then(|value| value.to_str().ok()),
        Some("E2E Profile")
    );
    assert_eq!(
        raw_response
            .headers()
            .get("profile-update-interval")
            .and_then(|value| value.to_str().ok()),
        Some("24")
    );
    assert!(
        raw_response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains(".json"))
    );
    assert_eq!(
        raw_response
            .headers()
            .get("subscription-userinfo")
            .and_then(|value| value.to_str().ok()),
        Some("upload=1; download=2; total=3; expire=1735689600")
    );
    let raw_payload = read_json(raw_response).await;
    let first_generated_at = raw_payload
        .get("generated_at")
        .and_then(Value::as_str)
        .expect("raw 缺少 generated_at")
        .to_string();
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

    let clash_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/clash?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建 clash 请求失败"),
        )
        .await
        .expect("读取 clash 订阅请求执行失败");
    assert_eq!(clash_response.status(), StatusCode::OK);
    assert_eq!(
        clash_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/yaml; charset=utf-8")
    );
    let clash_bytes = to_bytes(clash_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 clash 响应体失败");
    let clash_text = String::from_utf8(clash_bytes.to_vec()).expect("clash 响应不是 UTF-8");
    assert!(clash_text.contains("proxies:"));
    assert!(clash_text.contains("proxy-groups:"));

    let singbox_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/sing-box?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建 sing-box 请求失败"),
        )
        .await
        .expect("读取 sing-box 订阅请求执行失败");
    assert_eq!(singbox_response.status(), StatusCode::OK);
    assert_eq!(
        singbox_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json; charset=utf-8")
    );
    let singbox_json = read_json(singbox_response).await;
    assert!(singbox_json.get("outbounds").is_some());

    let base64_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/base64?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建 base64 请求失败"),
        )
        .await
        .expect("读取 base64 订阅请求执行失败");
    assert_eq!(base64_response.status(), StatusCode::OK);
    assert_eq!(
        base64_response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let base64_bytes = to_bytes(base64_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 base64 响应体失败");
    let base64_text = String::from_utf8(base64_bytes.to_vec()).expect("base64 响应不是 UTF-8");
    let decoded = BASE64_STANDARD
        .decode(base64_text.as_bytes())
        .expect("base64 响应应可解码");
    let decoded_text = String::from_utf8(decoded).expect("base64 解码内容不是 UTF-8");
    assert!(decoded_text.contains('\n'));

    let raw_cached_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/raw?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建第二次 raw 请求失败"),
        )
        .await
        .expect("读取第二次 raw 订阅请求执行失败");
    assert_eq!(raw_cached_response.status(), StatusCode::OK);
    let raw_cached_payload = read_json(raw_cached_response).await;
    let second_generated_at = raw_cached_payload
        .get("generated_at")
        .and_then(Value::as_str)
        .expect("第二次 raw 缺少 generated_at");
    assert_eq!(first_generated_at, second_generated_at);

    let refresh_profile_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/profiles/{profile_id}/refresh"),
            Body::empty(),
        ))
        .await
        .expect("刷新 profile 请求执行失败");
    assert_eq!(refresh_profile_response.status(), StatusCode::OK);

    let raw_after_refresh = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/raw?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建刷新后 raw 请求失败"),
        )
        .await
        .expect("读取刷新后 raw 订阅请求执行失败");
    assert_eq!(raw_after_refresh.status(), StatusCode::OK);
    let raw_after_refresh_payload = read_json(raw_after_refresh).await;
    let third_generated_at = raw_after_refresh_payload
        .get("generated_at")
        .and_then(Value::as_str)
        .expect("刷新后 raw 缺少 generated_at");
    assert_ne!(first_generated_at, third_generated_at);

    let source_b_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "E2E Source B",
                "config": {
                    "url": format!("{upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建第二个来源请求执行失败");
    assert_eq!(source_b_response.status(), StatusCode::CREATED);
    let source_b_payload = read_json(source_b_response).await;
    let source_b_id = source_b_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("第二个来源响应缺少 source.id")
        .to_string();

    let update_profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::PUT,
            &format!("/api/profiles/{profile_id}"),
            &json!({
                "source_ids": [source_id, source_b_id]
            }),
        ))
        .await
        .expect("更新 profile 来源列表请求执行失败");
    assert_eq!(update_profile_response.status(), StatusCode::OK);

    let multi_source_raw = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/raw?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建多来源 raw 请求失败"),
        )
        .await
        .expect("读取多来源 raw 请求执行失败");
    assert_eq!(multi_source_raw.status(), StatusCode::OK);
    assert!(
        multi_source_raw
            .headers()
            .get("subscription-userinfo")
            .is_none()
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

#[tokio::test]
async fn e2e_plugin_toggle_and_delete_workflow() {
    let state = build_test_state();
    let app = build_router(state);

    let boundary = "----subforge-e2e-plugin-toggle-boundary";
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
                .expect("构建插件导入请求失败"),
        )
        .await
        .expect("导入插件请求执行失败");
    assert_eq!(import_response.status(), StatusCode::CREATED);
    let import_payload = read_json(import_response).await;
    let plugin_id = import_payload
        .get("id")
        .and_then(Value::as_str)
        .expect("导入响应缺少插件 id")
        .to_string();

    let disable_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::PUT,
            &format!("/api/plugins/{plugin_id}/toggle"),
            &json!({ "enabled": false }),
        ))
        .await
        .expect("禁用插件请求执行失败");
    assert_eq!(disable_response.status(), StatusCode::OK);
    let disabled_payload = read_json(disable_response).await;
    assert_eq!(
        disabled_payload.get("status").and_then(Value::as_str),
        Some("disabled")
    );

    let enable_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::PUT,
            &format!("/api/plugins/{plugin_id}/toggle"),
            &json!({ "enabled": true }),
        ))
        .await
        .expect("启用插件请求执行失败");
    assert_eq!(enable_response.status(), StatusCode::OK);
    let enabled_payload = read_json(enable_response).await;
    assert_eq!(
        enabled_payload.get("status").and_then(Value::as_str),
        Some("enabled")
    );

    let source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "Plugin Toggle Source",
                "config": {
                    "url": "https://example.com/subscription"
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

    let delete_conflict = app
        .clone()
        .oneshot(admin_request(
            Method::DELETE,
            &format!("/api/plugins/{plugin_id}"),
            Body::empty(),
        ))
        .await
        .expect("删除插件冲突请求执行失败");
    assert_eq!(delete_conflict.status(), StatusCode::CONFLICT);

    let delete_source_response = app
        .clone()
        .oneshot(admin_request(
            Method::DELETE,
            &format!("/api/sources/{source_id}"),
            Body::empty(),
        ))
        .await
        .expect("删除来源请求执行失败");
    assert_eq!(delete_source_response.status(), StatusCode::OK);

    let delete_plugin_response = app
        .clone()
        .oneshot(admin_request(
            Method::DELETE,
            &format!("/api/plugins/{plugin_id}"),
            Body::empty(),
        ))
        .await
        .expect("删除插件请求执行失败");
    assert_eq!(delete_plugin_response.status(), StatusCode::OK);
}
