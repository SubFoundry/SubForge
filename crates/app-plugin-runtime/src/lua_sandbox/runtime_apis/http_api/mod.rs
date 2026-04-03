use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;

use app_transport::{NetworkProfileFactory, TransportProfile};
use mlua::{Error as LuaError, Lua, LuaSerdeExt, Table, Value as LuaValue};
use reqwest::{Method, Url};

use super::map_lua_error;
use super::{CookieStore, HttpRequestInput, HttpResponseOutput};
use super::{
    HTTP_REQUEST_LIMIT_SENTINEL, HTTP_RESPONSE_LIMIT_SENTINEL, SCRIPT_HTTP_MAX_REDIRECTS,
    SCRIPT_HTTP_MAX_REQUESTS, SCRIPT_HTTP_MAX_RESPONSE_BYTES, SCRIPT_HTTP_TIMEOUT_MS,
};
use crate::{PluginRuntimeError, PluginRuntimeResult};

mod cookies;
mod headers;
mod runtime;
mod target_guard;

pub(super) fn register_http_api(
    lua: &Lua,
    network_profile: &str,
    cookie_store: CookieStore,
    request_counter: std::sync::Arc<AtomicUsize>,
) -> PluginRuntimeResult<()> {
    let transport_profile = NetworkProfileFactory::create(network_profile)
        .map_err(|error| PluginRuntimeError::ScriptRuntime(error.to_string()))?;
    let http_table = lua.create_table().map_err(map_lua_error)?;

    let request_cookie_store = std::sync::Arc::clone(&cookie_store);
    let request_fn = lua
        .create_function(move |lua, request_table: Table| {
            let next = request_counter
                .fetch_add(1, AtomicOrdering::Relaxed)
                .saturating_add(1);
            if next > SCRIPT_HTTP_MAX_REQUESTS {
                return Err(LuaError::runtime(HTTP_REQUEST_LIMIT_SENTINEL));
            }

            let request: HttpRequestInput = lua.from_value(LuaValue::Table(request_table))?;
            let response = execute_http_request(
                transport_profile.as_ref(),
                request,
                std::sync::Arc::clone(&request_cookie_store),
            )?;
            lua.to_value(&response)
        })
        .map_err(map_lua_error)?;

    http_table
        .set("request", request_fn)
        .map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("http", http_table).map_err(map_lua_error)?;
    Ok(())
}

fn execute_http_request(
    transport_profile: &dyn TransportProfile,
    request: HttpRequestInput,
    cookie_store: CookieStore,
) -> Result<HttpResponseOutput, LuaError> {
    let url = Url::parse(request.url.trim())
        .map_err(|error| LuaError::runtime(format!("http.request url 非法：{error}")))?;
    target_guard::ensure_allowed_target(&url)?;

    let timeout_ms = request
        .timeout_ms
        .unwrap_or(SCRIPT_HTTP_TIMEOUT_MS)
        .min(SCRIPT_HTTP_TIMEOUT_MS);
    let timeout = Duration::from_millis(timeout_ms);
    let client = transport_profile
        .build_client_with_limits(timeout, SCRIPT_HTTP_MAX_REDIRECTS)
        .map_err(|error| LuaError::runtime(format!("http.request 客户端初始化失败：{error}")))?;

    let method = request
        .method
        .as_deref()
        .unwrap_or("GET")
        .parse::<Method>()
        .map_err(|error| LuaError::runtime(format!("http.request method 非法：{error}")))?;
    let headers = headers::build_request_headers(
        transport_profile,
        request.headers.as_ref(),
        std::sync::Arc::clone(&cookie_store),
    )?;

    let mut retry_attempt = 0usize;
    loop {
        if retry_attempt > 0 {
            thread::sleep(runtime::retry_backoff(
                transport_profile.request_delay(),
                retry_attempt,
            ));
        }

        let client_cloned = client.clone();
        let url_cloned = url.clone();
        let headers_cloned = headers.clone();
        let method_cloned = method.clone();
        let body = request.body.clone();

        let response = runtime::run_reqwest_blocking(async move {
            let mut request_builder = client_cloned
                .request(method_cloned, url_cloned)
                .headers(headers_cloned)
                .timeout(timeout);
            if let Some(body) = body {
                request_builder = request_builder.body(body);
            }

            let mut response = request_builder
                .send()
                .await
                .map_err(|error| format!("发送请求失败：{error}"))?;
            let status = response.status();
            let final_url = response.url().to_string();
            let response_headers = response.headers().clone();
            if let Some(content_length) = response.content_length()
                && content_length > SCRIPT_HTTP_MAX_RESPONSE_BYTES as u64
            {
                return Err(format!(
                    "响应体过大：{} bytes（限制 {} bytes）",
                    content_length, SCRIPT_HTTP_MAX_RESPONSE_BYTES
                ));
            }

            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| format!("读取响应体失败：{error}"))?
            {
                body.extend_from_slice(&chunk);
                if body.len() > SCRIPT_HTTP_MAX_RESPONSE_BYTES {
                    return Err(HTTP_RESPONSE_LIMIT_SENTINEL.to_string());
                }
            }
            Ok((status, final_url, response_headers, body))
        })
        .map_err(|error| LuaError::runtime(format!("http.request 失败：{error}")))?;

        let (status, final_url, response_headers, response_body) = response;
        cookies::apply_response_cookies(&response_headers, std::sync::Arc::clone(&cookie_store))?;
        if !status.is_success() {
            if retry_attempt < transport_profile.max_retries()
                && transport_profile.is_retryable_status(status)
            {
                retry_attempt += 1;
                continue;
            }
            return Err(LuaError::runtime(format!(
                "http.request 返回非成功状态码：{}",
                status.as_u16()
            )));
        }

        if response_body.len() > SCRIPT_HTTP_MAX_RESPONSE_BYTES {
            return Err(LuaError::runtime(HTTP_RESPONSE_LIMIT_SENTINEL));
        }

        let headers = headers::flatten_response_headers(&response_headers);
        let body = String::from_utf8_lossy(&response_body).to_string();
        return Ok(HttpResponseOutput {
            status: status.as_u16(),
            headers,
            body,
            final_url,
        });
    }
}
