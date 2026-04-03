use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use super::core_manager::CoreManager;
use super::helpers::{build_plugin_multipart_body, normalize_path};
use super::types::{CoreApiRequest, CoreApiResponse, MAX_PLUGIN_UPLOAD_BYTES, PluginImportRequest};

#[derive(Debug, Deserialize)]
struct SourceListPayload {
    sources: Vec<SourceRecordPayload>,
}

#[derive(Debug, Deserialize)]
struct SourceRecordPayload {
    source: SourceIdPayload,
}

#[derive(Debug, Deserialize)]
struct SourceIdPayload {
    id: String,
}

impl CoreManager {
    pub(crate) async fn proxy_api_call(&self, request: CoreApiRequest) -> Result<CoreApiResponse> {
        let (base_url, admin_token) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            self.try_restore_admin_token(&mut state);
            (state.base_url.clone(), state.admin_token.clone())
        };

        if self.fetch_health_version(&base_url).await.is_none() {
            return Err(anyhow!("Core 未运行或不可达"));
        }

        let path = normalize_path(&request.path);
        if is_admin_token_path(&path) {
            return Err(anyhow!(
                "安全策略限制：前端 IPC 不允许调用 /api/admin-token/*"
            ));
        }
        if path.starts_with("/api/") && admin_token.is_none() {
            return Err(anyhow!(
                "当前会话没有管理 token，请先通过 GUI 启动 Core 再调用管理 API"
            ));
        }

        let method = Method::from_bytes(request.method.as_bytes())
            .with_context(|| format!("不支持的 HTTP 方法: {}", request.method))?;
        let url = format!("{base_url}{path}");
        let response_redaction_token = admin_token.clone();

        let mut builder = self.client.request(method, &url);
        if let Some(token) = admin_token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }

        let response = builder
            .send()
            .await
            .with_context(|| format!("调用 Core API 失败: {url}"))?;
        let status = response.status().as_u16();

        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (key.to_string(), v.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let body = response.text().await.context("读取 Core API 响应失败")?;
        let (headers, body) =
            sanitize_core_response(headers, body, response_redaction_token.as_deref());

        Ok(CoreApiResponse {
            status,
            headers,
            body,
        })
    }

    pub(crate) async fn import_plugin_zip(
        &self,
        request: PluginImportRequest,
    ) -> Result<CoreApiResponse> {
        if !request.file_name.to_ascii_lowercase().ends_with(".zip") {
            return Err(anyhow!("仅支持 .zip 插件包"));
        }

        let payload = BASE64_STANDARD
            .decode(request.payload_base64.as_bytes())
            .context("解析插件包内容失败（Base64）")?;
        if payload.len() > MAX_PLUGIN_UPLOAD_BYTES {
            return Err(anyhow!(
                "插件包超过大小限制：{} bytes",
                MAX_PLUGIN_UPLOAD_BYTES
            ));
        }

        let (base_url, admin_token) = {
            let mut state = self.lock_state()?;
            self.reap_child_if_exited(&mut state)?;
            self.try_restore_admin_token(&mut state);
            (state.base_url.clone(), state.admin_token.clone())
        };

        if self.fetch_health_version(&base_url).await.is_none() {
            return Err(anyhow!("Core 未运行或不可达"));
        }

        let token = admin_token.ok_or_else(|| {
            anyhow!("当前会话没有管理 token，请先通过 GUI 启动 Core 再调用管理 API")
        })?;

        let boundary_seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let boundary = format!("----subforge-desktop-{boundary_seed}");
        let multipart_body = build_plugin_multipart_body(&boundary, &payload, &request.file_name);
        let response = self
            .client
            .request(Method::POST, format!("{base_url}/api/plugins/import"))
            .bearer_auth(&token)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(multipart_body)
            .send()
            .await
            .context("调用 Core 插件导入接口失败")?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (key.to_string(), v.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let body = response.text().await.context("读取插件导入响应失败")?;
        let (headers, body) = sanitize_core_response(headers, body, Some(token.as_str()));

        Ok(CoreApiResponse {
            status,
            headers,
            body,
        })
    }

    pub(crate) async fn refresh_all_sources(&self) -> Result<()> {
        let response = self
            .proxy_api_call(CoreApiRequest {
                method: "GET".to_string(),
                path: "/api/sources".to_string(),
                body: None,
            })
            .await?;
        if response.status != 200 {
            return Err(anyhow!(
                "读取来源列表失败，HTTP 状态码：{}",
                response.status
            ));
        }

        let payload: SourceListPayload =
            serde_json::from_str(&response.body).context("解析来源列表响应失败")?;
        for source in payload.sources {
            let refresh_path = format!("/api/sources/{}/refresh", source.source.id);
            let refresh_response = self
                .proxy_api_call(CoreApiRequest {
                    method: "POST".to_string(),
                    path: refresh_path,
                    body: None,
                })
                .await?;

            if !(200..300).contains(&refresh_response.status) {
                return Err(anyhow!(
                    "刷新来源失败，HTTP 状态码：{}",
                    refresh_response.status
                ));
            }
        }
        Ok(())
    }
}

fn is_admin_token_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    let route = normalized.split('?').next().unwrap_or_default();
    let lowered = route.to_ascii_lowercase();
    lowered == "/api/admin-token" || lowered.starts_with("/api/admin-token/")
}

const REDACTED_VALUE: &str = "***REDACTED***";

fn sanitize_core_response(
    mut headers: BTreeMap<String, String>,
    body: String,
    admin_token: Option<&str>,
) -> (BTreeMap<String, String>, String) {
    if let Some(token) = admin_token.filter(|token| !token.is_empty()) {
        for header_value in headers.values_mut() {
            if header_value.contains(token) {
                *header_value = header_value.replace(token, REDACTED_VALUE);
            }
        }
    }

    let mut sanitized_body = body;
    if let Some(token) = admin_token.filter(|token| !token.is_empty()) {
        if sanitized_body.contains(token) {
            sanitized_body = sanitized_body.replace(token, REDACTED_VALUE);
        }
    }

    if let Ok(mut payload) = serde_json::from_str::<Value>(&sanitized_body) {
        redact_admin_token_fields(&mut payload);
        if let Ok(serialized) = serde_json::to_string(&payload) {
            sanitized_body = serialized;
        }
    }

    (headers, sanitized_body)
}

fn redact_admin_token_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                let normalized_key = key.replace(['_', '-'], "");
                if normalized_key.eq_ignore_ascii_case("admintoken") {
                    *nested = Value::String(REDACTED_VALUE.to_string());
                    continue;
                }
                redact_admin_token_fields(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_admin_token_fields(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{REDACTED_VALUE, is_admin_token_path, sanitize_core_response};

    #[test]
    fn admin_token_paths_are_blocked() {
        assert!(is_admin_token_path("/api/admin-token"));
        assert!(is_admin_token_path("/api/admin-token/rotate"));
        assert!(is_admin_token_path("api/admin-token/status"));
        assert!(is_admin_token_path("/api/admin-token/rotate?from=desktop"));
        assert!(is_admin_token_path("/API/ADMIN-TOKEN/ROTATE"));
    }

    #[test]
    fn non_admin_token_paths_are_not_blocked() {
        assert!(!is_admin_token_path("/api/tokens/p-1/rotate"));
        assert!(!is_admin_token_path("/api/system/status"));
        assert!(!is_admin_token_path("/api/admin-tokenize"));
        assert!(!is_admin_token_path("/health"));
    }

    #[test]
    fn sanitize_response_redacts_admin_token_field_and_literal() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "x-debug".to_string(),
            "token=desktop-admin-token".to_string(),
        );

        let body = json!({
            "admin_token": "desktop-admin-token",
            "adminToken": "desktop-admin-token",
            "admin-token": "desktop-admin-token",
            "nested": { "Admin_Token": "desktop-admin-token" },
            "token": "export-token-should-remain"
        })
        .to_string();

        let (sanitized_headers, sanitized_body) =
            sanitize_core_response(headers, body, Some("desktop-admin-token"));

        assert_eq!(
            sanitized_headers.get("x-debug"),
            Some(&format!("token={REDACTED_VALUE}"))
        );

        let payload: serde_json::Value =
            serde_json::from_str(&sanitized_body).expect("响应必须仍为 JSON");
        assert_eq!(payload["admin_token"], REDACTED_VALUE);
        assert_eq!(payload["adminToken"], REDACTED_VALUE);
        assert_eq!(payload["admin-token"], REDACTED_VALUE);
        assert_eq!(payload["nested"]["Admin_Token"], REDACTED_VALUE);
        assert_eq!(payload["token"], "export-token-should-remain");
    }

    #[test]
    fn sanitize_response_redacts_admin_token_field_without_context_token() {
        let headers = BTreeMap::new();
        let body = r#"{"admin_token":"server-side-token","safe":"ok"}"#.to_string();
        let (_headers, sanitized_body) = sanitize_core_response(headers, body, None);
        let payload: serde_json::Value =
            serde_json::from_str(&sanitized_body).expect("响应必须仍为 JSON");

        assert_eq!(payload["admin_token"], REDACTED_VALUE);
        assert_eq!(payload["safe"], "ok");
    }

    #[test]
    fn sanitize_response_replaces_token_literal_in_plain_text_body() {
        let headers = BTreeMap::new();
        let body = "debug token: desktop-admin-token".to_string();
        let (_headers, sanitized_body) =
            sanitize_core_response(headers, body, Some("desktop-admin-token"));

        assert_eq!(sanitized_body, format!("debug token: {REDACTED_VALUE}"));
    }
}
