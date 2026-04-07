use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header::CONTENT_TYPE, header::HOST};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

const CLASH_TEMPLATE_FIXTURE: &str = r#"
mixed-port: 7890
mode: rule
dns:
  enable: true
  ipv6: false
proxies:
  - name: HK-Template-1
    type: trojan
    server: hk-template-1.example.com
    port: 443
    password: template-pass
  - name: HK-Template-2
    type: trojan
    server: hk-template-2.example.com
    port: 444
    password: template-pass
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - Auto
      - HK-Template-1
      - HK-Template-2
  - name: Auto
    type: url-test
    proxies:
      - HK-Template-1
      - HK-Template-2
    url: http://www.gstatic.com/generate_204
    interval: 300
    tolerance: 50
rules:
  - MATCH,Proxy
"#;

const SINGBOX_TEMPLATE_FIXTURE: &str = r#"
{
  "outbounds": [
    {
      "type": "selector",
      "tag": "Proxy",
      "outbounds": ["Auto", "DIRECT"]
    },
    {
      "type": "urltest",
      "tag": "Auto",
      "outbounds": ["old-node-a"],
      "url": "http://www.gstatic.com/generate_204",
      "interval": 300
    },
    {
      "type": "shadowsocks",
      "tag": "old-node-a",
      "server": "old-node.example.com",
      "server_port": 443,
      "method": "aes-128-gcm",
      "password": "p@ss"
    }
  ],
  "route": {
    "rules": [
      {
        "domain_suffix": ["example.com"],
        "outbound": "Proxy"
      }
    ],
    "final": "Proxy"
  }
}
"#;

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

    let plugin_schema_response = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            "/api/plugins/subforge.builtin.static/schema",
            Body::empty(),
        ))
        .await
        .expect("读取插件 schema 请求执行失败");
    assert_eq!(plugin_schema_response.status(), StatusCode::OK);
    let plugin_schema_payload = read_json(plugin_schema_response).await;
    assert_eq!(
        plugin_schema_payload
            .get("plugin_id")
            .and_then(Value::as_str),
        Some("subforge.builtin.static")
    );
    assert_eq!(
        plugin_schema_payload
            .pointer("/schema/type")
            .and_then(Value::as_str),
        Some("object")
    );

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
    let export_token_from_response = profile_payload
        .pointer("/profile/export_token")
        .and_then(Value::as_str)
        .expect("Profile 响应缺少 export_token")
        .to_string();

    let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
    let export_token = export_token_repository
        .get_active_token(&profile_id)
        .expect("读取 export_token 失败")
        .expect("创建 Profile 后应自动生成 export_token")
        .token;
    assert_eq!(export_token, export_token_from_response);

    let profiles_response = app
        .clone()
        .oneshot(admin_request(Method::GET, "/api/profiles", Body::empty()))
        .await
        .expect("读取 profile 列表失败");
    assert_eq!(profiles_response.status(), StatusCode::OK);
    let profiles_payload = read_json(profiles_response).await;
    let profiles = profiles_payload
        .get("profiles")
        .and_then(Value::as_array)
        .expect("profiles 响应缺少数组字段");
    assert!(profiles.iter().any(|item| {
        item.pointer("/profile/id").and_then(Value::as_str) == Some(profile_id.as_str())
            && item
                .get("export_token")
                .and_then(Value::as_str)
                .is_some_and(|token| token == export_token)
    }));

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
    assert_eq!(
        logs_payload
            .pointer("/pagination/limit")
            .and_then(Value::as_u64),
        Some(5)
    );
    assert_eq!(
        logs_payload
            .pointer("/pagination/offset")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert!(
        logs_payload
            .pointer("/pagination/total")
            .and_then(Value::as_u64)
            .is_some()
    );
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
    assert_eq!(
        failed_logs_payload
            .pointer("/pagination/limit")
            .and_then(Value::as_u64),
        Some(5)
    );

    let filtered_logs_response = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            &format!("/api/logs?status=failed&source_id={source_id}&limit=1&offset=0"),
            Body::empty(),
        ))
        .await
        .expect("读取按来源过滤 logs 失败");
    assert_eq!(filtered_logs_response.status(), StatusCode::OK);
    let filtered_logs_payload = read_json(filtered_logs_response).await;
    let filtered_logs = filtered_logs_payload
        .get("logs")
        .and_then(Value::as_array)
        .expect("filtered logs 响应缺少数组字段");
    assert_eq!(filtered_logs.len(), 1);
    assert!(filtered_logs.iter().all(|entry| {
        entry.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
            && entry.get("status").and_then(Value::as_str) == Some("failed")
    }));
    assert_eq!(
        filtered_logs_payload
            .pointer("/pagination/offset")
            .and_then(Value::as_u64),
        Some(0)
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

    let refresh_source_b_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/sources/{source_b_id}/refresh"),
            Body::empty(),
        ))
        .await
        .expect("刷新第二个来源请求执行失败");
    assert_eq!(refresh_source_b_response.status(), StatusCode::OK);
    let refresh_source_b_payload = read_json(refresh_source_b_response).await;
    assert_eq!(
        refresh_source_b_payload
            .get("source_id")
            .and_then(Value::as_str),
        Some(source_b_id.as_str())
    );
    assert_eq!(
        refresh_source_b_payload
            .get("node_count")
            .and_then(Value::as_u64),
        Some(3)
    );

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
    let multi_source_payload = read_json(multi_source_raw).await;
    assert_eq!(
        multi_source_payload
            .get("node_count")
            .and_then(Value::as_u64),
        Some(3),
        "多来源重复节点应去重后保持 3 个"
    );
    assert_eq!(
        multi_source_payload
            .get("nodes")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(3),
        "重复来源刷新后仍应输出去重后的 3 条节点"
    );

    server_task.abort();
}

#[tokio::test]
async fn e2e_profile_clash_template_source_applies_template_groups() {
    let state = build_test_state();
    let app = build_router(state);

    let (template_upstream_base, template_server_task) = start_fixture_server(
        CLASH_TEMPLATE_FIXTURE.trim().to_string(),
        "text/yaml; charset=utf-8",
    )
    .await;
    let (nodes_upstream_base, nodes_server_task) = start_fixture_server(
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let boundary = "----subforge-e2e-template-boundary";
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

    let template_source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "Template Source",
                "config": {
                    "url": format!("{template_upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建模板来源失败");
    assert_eq!(template_source_response.status(), StatusCode::CREATED);
    let template_source_payload = read_json(template_source_response).await;
    let template_source_id = template_source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("模板来源缺少 id")
        .to_string();

    let nodes_source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "Nodes Source",
                "config": {
                    "url": format!("{nodes_upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建节点来源失败");
    assert_eq!(nodes_source_response.status(), StatusCode::CREATED);
    let nodes_source_payload = read_json(nodes_source_response).await;
    let nodes_source_id = nodes_source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("节点来源缺少 id")
        .to_string();

    for source_id in [template_source_id.as_str(), nodes_source_id.as_str()] {
        let refresh_response = app
            .clone()
            .oneshot(admin_request(
                Method::POST,
                &format!("/api/sources/{source_id}/refresh"),
                Body::empty(),
            ))
            .await
            .expect("刷新来源失败");
        assert_eq!(refresh_response.status(), StatusCode::OK);
    }

    let profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/profiles",
            &json!({
                "name": "Template Profile",
                "source_ids": [template_source_id.clone(), nodes_source_id.clone()],
                "routing_template_source_id": template_source_id,
            }),
        ))
        .await
        .expect("创建 Profile 失败");
    assert_eq!(profile_response.status(), StatusCode::CREATED);
    let profile_payload = read_json(profile_response).await;
    let profile_id = profile_payload
        .pointer("/profile/profile/id")
        .and_then(Value::as_str)
        .expect("Profile 缺少 id")
        .to_string();
    let export_token = profile_payload
        .pointer("/profile/export_token")
        .and_then(Value::as_str)
        .expect("Profile 缺少 export_token")
        .to_string();

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
        .expect("获取 clash 订阅失败");
    assert_eq!(clash_response.status(), StatusCode::OK);
    let clash_bytes = to_bytes(clash_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 clash 响应体失败");
    let clash_text = String::from_utf8(clash_bytes.to_vec()).expect("clash 响应不是 UTF-8");
    assert!(clash_text.contains("mixed-port: 7890"));
    assert!(clash_text.contains("dns:"));
    assert!(clash_text.contains("proxy-groups:"));
    assert!(clash_text.contains("\nrules:"));
    assert!(clash_text.contains("- MATCH,Proxy"));

    assert!(clash_text.contains("- name: Proxy"));
    assert!(clash_text.contains("- name: Auto"));
    assert!(clash_text.contains("name: HK-Template-1"));
    assert!(clash_text.contains("name: HK-Template-2"));

    let proxy_group_start = clash_text.find("- name: Proxy").expect("缺少 Proxy 分组块");
    let proxy_group_tail = &clash_text[proxy_group_start..];
    let proxy_group_end = proxy_group_tail
        .get(1..)
        .and_then(|tail| tail.find("\n- name: ").map(|index| index + 1))
        .unwrap_or(proxy_group_tail.len());
    let proxy_group_block = &proxy_group_tail[..proxy_group_end];
    assert!(proxy_group_block.contains("- Auto"));
    assert!(proxy_group_block.contains("- HK-Template-1"));
    assert!(proxy_group_block.contains("- HK-Template-2"));
    assert!(proxy_group_block.contains("- HK-SS"));
    assert!(proxy_group_block.contains("- SG-VMESS"));
    assert!(proxy_group_block.contains("- US-Trojan"));

    let auto_group_start = clash_text.find("- name: Auto").expect("缺少 Auto 分组块");
    let auto_group_tail = &clash_text[auto_group_start..];
    let auto_group_end = auto_group_tail
        .get(1..)
        .and_then(|tail| tail.find("\n- name: ").map(|index| index + 1))
        .unwrap_or(auto_group_tail.len());
    let auto_group_block = &auto_group_tail[..auto_group_end];
    assert!(auto_group_block.contains("- HK-Template-1"));
    assert!(auto_group_block.contains("- HK-Template-2"));
    assert!(auto_group_block.contains("- HK-SS"));
    assert!(auto_group_block.contains("- SG-VMESS"));
    assert!(auto_group_block.contains("- US-Trojan"));

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
        .expect("获取 sing-box 订阅失败");
    assert_eq!(singbox_response.status(), StatusCode::OK);
    let singbox_payload = read_json(singbox_response).await;
    let outbounds = singbox_payload
        .get("outbounds")
        .and_then(Value::as_array)
        .expect("sing-box 响应缺少 outbounds");
    let proxy_selector = outbounds
        .iter()
        .find(|outbound| outbound.get("tag").and_then(Value::as_str) == Some("Proxy"))
        .expect("sing-box 响应缺少 Proxy selector");
    let proxy_targets = proxy_selector
        .get("outbounds")
        .and_then(Value::as_array)
        .expect("Proxy selector 缺少 outbounds")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(proxy_targets.contains(&"Auto"));
    assert!(proxy_targets.contains(&"HK-Template-1"));
    assert!(proxy_targets.contains(&"HK-Template-2"));
    assert!(proxy_targets.contains(&"HK-SS"));
    assert!(proxy_targets.contains(&"SG-VMESS"));
    assert!(proxy_targets.contains(&"US-Trojan"));

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
        .expect("获取 raw 订阅失败");
    assert_eq!(raw_response.status(), StatusCode::OK);
    let raw_payload = read_json(raw_response).await;
    let raw_nodes = raw_payload
        .get("nodes")
        .and_then(Value::as_array)
        .expect("raw 响应缺少 nodes");
    assert_eq!(raw_nodes.len(), 5, "raw 应保留母版节点并聚合其它来源节点");
    let raw_node_names = raw_nodes
        .iter()
        .filter_map(|node| node.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(raw_node_names.contains(&"HK-Template-1"));
    assert!(raw_node_names.contains(&"HK-Template-2"));
    assert!(raw_node_names.contains(&"HK-SS"));
    assert!(raw_node_names.contains(&"SG-VMESS"));
    assert!(raw_node_names.contains(&"US-Trojan"));

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
        .expect("获取 base64 订阅失败");
    assert_eq!(base64_response.status(), StatusCode::OK);
    let base64_bytes = to_bytes(base64_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 base64 响应体失败");
    let base64_text = String::from_utf8(base64_bytes.to_vec()).expect("base64 响应不是 UTF-8");
    let decoded_base64 = BASE64_STANDARD
        .decode(base64_text.as_bytes())
        .expect("base64 响应应可解码");
    let decoded_base64_text = String::from_utf8(decoded_base64).expect("base64 解码内容不是 UTF-8");
    let base64_lines = decoded_base64_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(base64_lines.len(), 5, "base64 应导出最终聚合后的全部节点");
    assert!(
        decoded_base64_text.contains("hk-template-1.example.com"),
        "base64 应包含 Clash 母版节点"
    );
    assert!(
        decoded_base64_text.contains("hk-template-2.example.com"),
        "base64 应包含 Clash 母版节点"
    );
    assert!(
        decoded_base64_text.contains("us.example.com"),
        "base64 应包含聚合来源节点"
    );

    // 删除模板来源后，Profile 应回退为默认分组导出（Select/Auto/Region）。
    let delete_template_source_response = app
        .clone()
        .oneshot(admin_request(
            Method::DELETE,
            &format!("/api/sources/{template_source_id}"),
            Body::empty(),
        ))
        .await
        .expect("删除模板来源失败");
    assert_eq!(delete_template_source_response.status(), StatusCode::OK);

    let clash_fallback_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/profiles/{profile_id}/clash?token={export_token}"
                ))
                .header(HOST, "127.0.0.1:18118")
                .body(Body::empty())
                .expect("构建回退 clash 请求失败"),
        )
        .await
        .expect("获取回退 clash 订阅失败");
    assert_eq!(clash_fallback_response.status(), StatusCode::OK);
    let clash_fallback_bytes = to_bytes(clash_fallback_response.into_body(), 1024 * 1024)
        .await
        .expect("读取回退 clash 响应体失败");
    let clash_fallback_text =
        String::from_utf8(clash_fallback_bytes.to_vec()).expect("回退 clash 响应不是 UTF-8");
    assert!(clash_fallback_text.contains("proxy-groups:"));
    assert!(!clash_fallback_text.contains("\nrules:"));
    assert!(!clash_fallback_text.contains("mixed-port: 7890"));
    assert!(clash_fallback_text.contains("- name: Select"));
    assert!(clash_fallback_text.contains("- name: Auto"));
    assert!(clash_fallback_text.contains("- name: HK"));
    assert!(clash_fallback_text.contains("- name: SG"));
    assert!(clash_fallback_text.contains("- name: US"));
    assert!(!clash_fallback_text.contains("- name: Proxy"));

    template_server_task.abort();
    nodes_server_task.abort();
}

#[tokio::test]
async fn e2e_profile_singbox_template_source_converts_to_clash_groups_and_rules() {
    let state = build_test_state();
    let app = build_router(state);

    let (template_upstream_base, template_server_task) = start_fixture_server(
        SINGBOX_TEMPLATE_FIXTURE.trim().to_string(),
        "application/json; charset=utf-8",
    )
    .await;
    let (nodes_upstream_base, nodes_server_task) = start_fixture_server(
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let boundary = "----subforge-e2e-singbox-template-boundary";
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

    let template_source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "Singbox Template Source",
                "config": {
                    "url": format!("{template_upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建 sing-box 模板来源失败");
    assert_eq!(template_source_response.status(), StatusCode::CREATED);
    let template_source_payload = read_json(template_source_response).await;
    let template_source_id = template_source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("sing-box 模板来源缺少 id")
        .to_string();

    let nodes_source_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/sources",
            &json!({
                "plugin_id": "subforge.builtin.static",
                "name": "Nodes Source",
                "config": {
                    "url": format!("{nodes_upstream_base}/sub")
                }
            }),
        ))
        .await
        .expect("创建节点来源失败");
    assert_eq!(nodes_source_response.status(), StatusCode::CREATED);
    let nodes_source_payload = read_json(nodes_source_response).await;
    let nodes_source_id = nodes_source_payload
        .pointer("/source/source/id")
        .and_then(Value::as_str)
        .expect("节点来源缺少 id")
        .to_string();

    for source_id in [template_source_id.as_str(), nodes_source_id.as_str()] {
        let refresh_response = app
            .clone()
            .oneshot(admin_request(
                Method::POST,
                &format!("/api/sources/{source_id}/refresh"),
                Body::empty(),
            ))
            .await
            .expect("刷新来源失败");
        assert_eq!(refresh_response.status(), StatusCode::OK);
    }

    let profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/profiles",
            &json!({
                "name": "Singbox Template Profile",
                "source_ids": [template_source_id.clone(), nodes_source_id.clone()],
                "routing_template_source_id": template_source_id,
            }),
        ))
        .await
        .expect("创建 Profile 失败");
    assert_eq!(profile_response.status(), StatusCode::CREATED);
    let profile_payload = read_json(profile_response).await;
    let profile_id = profile_payload
        .pointer("/profile/profile/id")
        .and_then(Value::as_str)
        .expect("Profile 缺少 id")
        .to_string();
    let export_token = profile_payload
        .pointer("/profile/export_token")
        .and_then(Value::as_str)
        .expect("Profile 缺少 export_token")
        .to_string();

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
        .expect("获取 clash 订阅失败");
    assert_eq!(clash_response.status(), StatusCode::OK);
    let clash_bytes = to_bytes(clash_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 clash 响应体失败");
    let clash_text = String::from_utf8(clash_bytes.to_vec()).expect("clash 响应不是 UTF-8");

    assert!(clash_text.contains("proxy-groups:"));
    assert!(clash_text.contains("\nrules:"));
    assert!(clash_text.contains("- name: Proxy"));
    assert!(clash_text.contains("- name: Auto"));
    assert!(clash_text.contains("- DOMAIN-SUFFIX,example.com,Proxy"));
    assert!(clash_text.contains("- MATCH,Proxy"));
    assert!(clash_text.contains("name: old-node-a"));

    let proxy_group_start = clash_text.find("- name: Proxy").expect("缺少 Proxy 分组块");
    let proxy_group_tail = &clash_text[proxy_group_start..];
    let proxy_group_end = proxy_group_tail
        .get(1..)
        .and_then(|tail| tail.find("\n- name: ").map(|index| index + 1))
        .unwrap_or(proxy_group_tail.len());
    let proxy_group_block = &proxy_group_tail[..proxy_group_end];
    assert!(proxy_group_block.contains("- Auto"));
    assert!(!proxy_group_block.contains("- old-node-a"));
    assert!(!proxy_group_block.contains("- HK-SS"));
    assert!(!proxy_group_block.contains("- SG-VMESS"));
    assert!(!proxy_group_block.contains("- US-Trojan"));

    let auto_group_start = clash_text.find("- name: Auto").expect("缺少 Auto 分组块");
    let auto_group_tail = &clash_text[auto_group_start..];
    let auto_group_end = auto_group_tail
        .get(1..)
        .and_then(|tail| tail.find("\n- name: ").map(|index| index + 1))
        .unwrap_or(auto_group_tail.len());
    let auto_group_block = &auto_group_tail[..auto_group_end];
    assert!(auto_group_block.contains("- old-node-a"));
    assert!(auto_group_block.contains("- HK-SS"));
    assert!(auto_group_block.contains("- SG-VMESS"));
    assert!(auto_group_block.contains("- US-Trojan"));

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
        .expect("获取 sing-box 订阅失败");
    assert_eq!(singbox_response.status(), StatusCode::OK);
    let singbox_payload = read_json(singbox_response).await;
    assert!(
        singbox_payload.get("route").is_none(),
        "sing-box 导出不应暴露 route"
    );
    assert!(
        singbox_payload.get("dns").is_none(),
        "sing-box 导出不应暴露 dns"
    );
    let singbox_outbounds = singbox_payload
        .get("outbounds")
        .and_then(Value::as_array)
        .expect("sing-box 响应缺少 outbounds");
    let singbox_proxy_selector = singbox_outbounds
        .iter()
        .find(|outbound| outbound.get("tag").and_then(Value::as_str) == Some("Proxy"))
        .expect("sing-box 响应缺少 Proxy selector");
    let singbox_proxy_targets = singbox_proxy_selector
        .get("outbounds")
        .and_then(Value::as_array)
        .expect("Proxy selector 缺少 outbounds")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(singbox_proxy_targets.contains(&"Auto"));
    assert!(singbox_proxy_targets.contains(&"direct"));
    assert!(!singbox_proxy_targets.contains(&"old-node-a"));
    assert!(!singbox_proxy_targets.contains(&"HK-SS"));
    assert!(!singbox_proxy_targets.contains(&"SG-VMESS"));
    assert!(!singbox_proxy_targets.contains(&"US-Trojan"));

    let singbox_auto_group = singbox_outbounds
        .iter()
        .find(|outbound| outbound.get("tag").and_then(Value::as_str) == Some("Auto"))
        .expect("sing-box 响应缺少 Auto urltest");
    let singbox_auto_targets = singbox_auto_group
        .get("outbounds")
        .and_then(Value::as_array)
        .expect("Auto urltest 缺少 outbounds")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(singbox_auto_targets.contains(&"old-node-a"));
    assert!(singbox_auto_targets.contains(&"HK-SS"));
    assert!(singbox_auto_targets.contains(&"SG-VMESS"));
    assert!(singbox_auto_targets.contains(&"US-Trojan"));

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
        .expect("获取 raw 订阅失败");
    assert_eq!(raw_response.status(), StatusCode::OK);
    let raw_payload = read_json(raw_response).await;
    let raw_nodes = raw_payload
        .get("nodes")
        .and_then(Value::as_array)
        .expect("raw 响应缺少 nodes");
    assert_eq!(
        raw_nodes.len(),
        4,
        "raw 应保留 sing-box 母版节点并聚合其它来源节点"
    );
    let raw_node_names = raw_nodes
        .iter()
        .filter_map(|node| node.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(raw_node_names.contains(&"old-node-a"));
    assert!(raw_node_names.contains(&"HK-SS"));
    assert!(raw_node_names.contains(&"SG-VMESS"));
    assert!(raw_node_names.contains(&"US-Trojan"));

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
        .expect("获取 base64 订阅失败");
    assert_eq!(base64_response.status(), StatusCode::OK);
    let base64_bytes = to_bytes(base64_response.into_body(), 1024 * 1024)
        .await
        .expect("读取 base64 响应体失败");
    let base64_text = String::from_utf8(base64_bytes.to_vec()).expect("base64 响应不是 UTF-8");
    let decoded_base64 = BASE64_STANDARD
        .decode(base64_text.as_bytes())
        .expect("base64 响应应可解码");
    let decoded_base64_text = String::from_utf8(decoded_base64).expect("base64 解码内容不是 UTF-8");
    let base64_lines = decoded_base64_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(base64_lines.len(), 4, "base64 应导出最终聚合后的全部节点");
    assert!(
        decoded_base64_text.contains("old-node.example.com"),
        "base64 应包含 sing-box 母版节点"
    );
    assert!(
        decoded_base64_text.contains("us.example.com"),
        "base64 应包含聚合来源节点"
    );

    template_server_task.abort();
    nodes_server_task.abort();
}

#[tokio::test]
async fn e2e_import_plugin_zip_with_top_level_directory() {
    let state = build_test_state();
    let app = build_router(state);

    let boundary = "----subforge-e2e-nested-plugin-boundary";
    let plugin_zip = build_builtin_plugin_zip_bytes_with_root_dir("builtin-static");
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
    let payload = read_json(import_response).await;
    assert_eq!(
        payload.get("plugin_id").and_then(Value::as_str),
        Some("subforge.builtin.static")
    );
}

#[tokio::test]
async fn e2e_import_plugin_zip_with_backslash_path_separator() {
    let state = build_test_state();
    let app = build_router(state);

    let boundary = "----subforge-e2e-backslash-path-boundary";
    let plugin_zip = build_builtin_plugin_zip_bytes_with_backslash_root_dir("builtin-static");
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
    let payload = read_json(import_response).await;
    assert_eq!(
        payload.get("plugin_id").and_then(Value::as_str),
        Some("subforge.builtin.static")
    );
}

#[tokio::test]
async fn e2e_refresh_source_with_unsupported_network_profile_returns_bad_request() {
    let state = build_test_state();
    let app = build_router(state.clone());

    let boundary = "----subforge-e2e-unsupported-profile-boundary";
    let plugin_zip = build_builtin_plugin_zip_bytes_with_network_profile("browser_firefox");
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
                "name": "Unsupported Profile Source",
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

    let refresh_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/sources/{source_id}/refresh"),
            Body::empty(),
        ))
        .await
        .expect("刷新来源请求执行失败");
    assert_eq!(refresh_response.status(), StatusCode::BAD_REQUEST);
    let refresh_payload = read_json(refresh_response).await;
    assert_eq!(
        refresh_payload.get("code").and_then(Value::as_str),
        Some("E_CONFIG_INVALID")
    );
    assert!(
        refresh_payload
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("network_profile"))
    );

    let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
    let refresh_jobs = refresh_repository
        .list_by_source(&source_id)
        .expect("读取 refresh_jobs 失败");
    assert_eq!(refresh_jobs.len(), 1);
    assert_eq!(refresh_jobs[0].status, "failed");
    assert_eq!(
        refresh_jobs[0].error_code.as_deref(),
        Some("E_CONFIG_INVALID")
    );
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

    let logs_response = app
        .clone()
        .oneshot(admin_request(
            Method::GET,
            &format!("/api/logs?source_id={source_id}&limit=5&include_script_logs=true"),
            Body::empty(),
        ))
        .await
        .expect("读取脚本来源 logs 失败");
    assert_eq!(logs_response.status(), StatusCode::OK);
    let logs_payload = read_json(logs_response).await;
    let logs = logs_payload
        .get("logs")
        .and_then(Value::as_array)
        .expect("脚本 logs 响应缺少数组字段");
    assert!(!logs.is_empty());
    let first = logs[0].as_object().expect("日志项应为对象");
    let script_logs = first
        .get("script_logs")
        .and_then(Value::as_array)
        .expect("include_script_logs=true 时应返回 script_logs");
    assert!(!script_logs.is_empty());
    assert!(script_logs.iter().all(|entry| {
        entry.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
    }));
    assert!(script_logs.iter().any(|entry| {
        entry
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("script-mock fetch subscription"))
    }));

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
async fn e2e_rotate_profile_export_token_supports_grace_period() {
    let state = build_test_state();
    let app = build_router(state.clone());

    let profile_response = app
        .clone()
        .oneshot(admin_json_request(
            Method::POST,
            "/api/profiles",
            &json!({
                "name": "Rotate Token Profile",
                "source_ids": []
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
    let old_token = profile_payload
        .pointer("/profile/export_token")
        .and_then(Value::as_str)
        .expect("Profile 响应缺少 export_token")
        .to_string();

    let rotate_response = app
        .clone()
        .oneshot(admin_request(
            Method::POST,
            &format!("/api/tokens/{profile_id}/rotate"),
            Body::empty(),
        ))
        .await
        .expect("轮换 token 请求执行失败");
    assert_eq!(rotate_response.status(), StatusCode::OK);
    let rotate_payload = read_json(rotate_response).await;
    let new_token = rotate_payload
        .get("token")
        .and_then(Value::as_str)
        .expect("轮换响应缺少 token")
        .to_string();
    let previous_expires_at = rotate_payload
        .get("previous_token_expires_at")
        .and_then(Value::as_str)
        .expect("轮换响应缺少 previous_token_expires_at")
        .to_string();
    assert_ne!(old_token, new_token);
    assert_eq!(new_token.len(), 43);

    let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
    let active = export_token_repository
        .get_active_token(&profile_id)
        .expect("读取 active token 失败")
        .expect("轮换后应存在 active token");
    assert_eq!(active.token, new_token);
    assert!(
        export_token_repository
            .is_valid_token(&profile_id, &old_token, "1970-01-01T00:00:00Z")
            .expect("校验旧 token 失败")
    );
    assert!(
        !export_token_repository
            .is_valid_token(&profile_id, &old_token, &previous_expires_at)
            .expect("校验旧 token 过期失败")
    );

    let profiles_response = app
        .clone()
        .oneshot(admin_request(Method::GET, "/api/profiles", Body::empty()))
        .await
        .expect("读取 profile 列表失败");
    assert_eq!(profiles_response.status(), StatusCode::OK);
    let profiles_payload = read_json(profiles_response).await;
    let profiles = profiles_payload
        .get("profiles")
        .and_then(Value::as_array)
        .expect("profiles 响应缺少数组字段");
    assert!(profiles.iter().any(|item| {
        item.pointer("/profile/id").and_then(Value::as_str) == Some(profile_id.as_str())
            && item
                .get("export_token")
                .and_then(Value::as_str)
                .is_some_and(|token| token == new_token)
    }));
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
