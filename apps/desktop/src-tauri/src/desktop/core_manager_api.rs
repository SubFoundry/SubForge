use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use reqwest::Method;
use serde::Deserialize;

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
        if path.starts_with("/api/") && admin_token.is_none() {
            return Err(anyhow!(
                "当前会话没有管理 token，请先通过 GUI 启动 Core 再调用管理 API"
            ));
        }

        let method = Method::from_bytes(request.method.as_bytes())
            .with_context(|| format!("不支持的 HTTP 方法: {}", request.method))?;
        let url = format!("{base_url}{path}");

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
            .bearer_auth(token)
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
