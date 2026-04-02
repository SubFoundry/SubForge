use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use app_secrets::SecretStore;
use app_transport::{NetworkProfileFactory, TransportProfile};
use base64::Engine;
use mlua::{Error as LuaError, Lua, LuaSerdeExt, Table, Value as LuaValue};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::runtime::Builder as TokioRuntimeBuilder;

use super::error_map::{map_lua_error, map_secret_error};
use super::{
    HTTP_REQUEST_LIMIT_SENTINEL, HTTP_RESPONSE_LIMIT_SENTINEL, LOG_PREFIX, LuaSandboxConfig,
    SCRIPT_HTTP_MAX_REDIRECTS, SCRIPT_HTTP_MAX_REQUESTS, SCRIPT_HTTP_MAX_RESPONSE_BYTES,
    SCRIPT_HTTP_TIMEOUT_MS,
};
use crate::{PluginRuntimeError, PluginRuntimeResult};

type CookieStore = Arc<Mutex<HashMap<String, CookieEntry>>>;

pub(super) fn new_cookie_store() -> CookieStore {
    Arc::new(Mutex::new(HashMap::new()))
}

#[derive(Debug, Deserialize)]
struct HttpRequestInput {
    url: String,
    method: Option<String>,
    headers: Option<std::collections::BTreeMap<String, String>>,
    body: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct CookieEntry {
    value: String,
    attrs: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct HttpResponseOutput {
    status: u16,
    headers: std::collections::BTreeMap<String, String>,
    body: String,
    final_url: String,
}

pub(super) fn register_runtime_apis(
    lua: &Lua,
    config: &LuaSandboxConfig,
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
    secret_scope: String,
    request_counter: Arc<AtomicUsize>,
) -> PluginRuntimeResult<()> {
    register_json_api(lua)?;
    register_base64_api(lua)?;
    register_time_api(lua)?;
    register_log_api(lua)?;
    register_html_api(lua)?;
    register_cookie_api(lua, Arc::clone(&cookie_store))?;
    register_secret_api(lua, Arc::clone(&config.secret_store), secret_scope)?;
    register_http_api(lua, &config.network_profile, cookie_store, request_counter)?;
    Ok(())
}

fn register_json_api(lua: &Lua) -> PluginRuntimeResult<()> {
    let json_table = lua.create_table().map_err(map_lua_error)?;
    let parse_fn = lua
        .create_function(|lua, payload: String| {
            let value: Value = serde_json::from_str(&payload)
                .map_err(|error| LuaError::runtime(format!("json.parse 失败：{error}")))?;
            lua.to_value(&value)
        })
        .map_err(map_lua_error)?;
    let stringify_fn = lua
        .create_function(|lua, payload: LuaValue| {
            let value: Value = lua.from_value(payload)?;
            serde_json::to_string(&value)
                .map_err(|error| LuaError::runtime(format!("json.stringify 失败：{error}")))
        })
        .map_err(map_lua_error)?;

    json_table.set("parse", parse_fn).map_err(map_lua_error)?;
    json_table
        .set("stringify", stringify_fn)
        .map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("json", json_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_base64_api(lua: &Lua) -> PluginRuntimeResult<()> {
    let base64_table = lua.create_table().map_err(map_lua_error)?;
    let encode_fn = lua
        .create_function(|_, payload: String| {
            Ok(base64::engine::general_purpose::STANDARD.encode(payload.as_bytes()))
        })
        .map_err(map_lua_error)?;
    let decode_fn = lua
        .create_function(|_, payload: String| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|error| LuaError::runtime(format!("base64.decode 失败：{error}")))?;
            String::from_utf8(bytes)
                .map_err(|error| LuaError::runtime(format!("base64.decode 非 UTF-8 文本：{error}")))
        })
        .map_err(map_lua_error)?;

    base64_table
        .set("encode", encode_fn)
        .map_err(map_lua_error)?;
    base64_table
        .set("decode", decode_fn)
        .map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("base64", base64_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_time_api(lua: &Lua) -> PluginRuntimeResult<()> {
    let time_table = lua.create_table().map_err(map_lua_error)?;
    let now_fn = lua
        .create_function(|_, ()| {
            let now = OffsetDateTime::now_utc();
            now.format(&Rfc3339)
                .map_err(|error| LuaError::runtime(format!("time.now 格式化失败：{error}")))
        })
        .map_err(map_lua_error)?;
    time_table.set("now", now_fn).map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("time", time_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_log_api(lua: &Lua) -> PluginRuntimeResult<()> {
    let log_table = lua.create_table().map_err(map_lua_error)?;
    let info_fn = lua
        .create_function(|_, message: String| {
            eprintln!("INFO: {} {}", LOG_PREFIX, message);
            Ok(())
        })
        .map_err(map_lua_error)?;
    let warn_fn = lua
        .create_function(|_, message: String| {
            eprintln!("WARN: {} {}", LOG_PREFIX, message);
            Ok(())
        })
        .map_err(map_lua_error)?;
    let error_fn = lua
        .create_function(|_, message: String| {
            eprintln!("ERROR: {} {}", LOG_PREFIX, message);
            Ok(())
        })
        .map_err(map_lua_error)?;

    log_table.set("info", info_fn).map_err(map_lua_error)?;
    log_table.set("warn", warn_fn).map_err(map_lua_error)?;
    log_table.set("error", error_fn).map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("log", log_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_html_api(lua: &Lua) -> PluginRuntimeResult<()> {
    let html_table = lua.create_table().map_err(map_lua_error)?;
    let query_fn = lua
        .create_function(|lua, (raw_html, selector): (String, String)| {
            let selector = Selector::parse(selector.trim())
                .map_err(|error| LuaError::runtime(format!("html.query selector 非法：{error}")))?;
            let document = Html::parse_document(&raw_html);
            let mut matches = Vec::new();
            for node in document.select(&selector) {
                let text = normalize_html_text(node.text().collect::<Vec<_>>().join(" "));
                matches.push(text);
            }
            lua.to_value(&matches)
        })
        .map_err(map_lua_error)?;
    html_table.set("query", query_fn).map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("html", html_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_cookie_api(
    lua: &Lua,
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
) -> PluginRuntimeResult<()> {
    let cookie_table = lua.create_table().map_err(map_lua_error)?;

    let get_store = Arc::clone(&cookie_store);
    let get_fn = lua
        .create_function(move |_, name: String| {
            let jar = get_store
                .lock()
                .map_err(|_| LuaError::runtime("cookie.get 无法获取会话锁"))?;
            Ok(jar.get(name.trim()).map(|entry| entry.value.clone()))
        })
        .map_err(map_lua_error)?;

    let set_store = Arc::clone(&cookie_store);
    let set_fn = lua
        .create_function(
            move |_, (name, value, attrs): (String, String, Option<Table>)| {
                let mut jar = set_store
                    .lock()
                    .map_err(|_| LuaError::runtime("cookie.set 无法获取会话锁"))?;
                jar.insert(
                    name.trim().to_string(),
                    CookieEntry {
                        value,
                        attrs: parse_cookie_attrs(attrs)?,
                    },
                );
                Ok(())
            },
        )
        .map_err(map_lua_error)?;

    cookie_table.set("get", get_fn).map_err(map_lua_error)?;
    cookie_table.set("set", set_fn).map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("cookie", cookie_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_secret_api(
    lua: &Lua,
    secret_store: Arc<dyn SecretStore>,
    secret_scope: String,
) -> PluginRuntimeResult<()> {
    let secret_table = lua.create_table().map_err(map_lua_error)?;

    let get_store = Arc::clone(&secret_store);
    let get_scope = secret_scope.clone();
    let get_fn = lua
        .create_function(move |_, key: String| {
            let secret = get_store
                .get(&get_scope, key.trim())
                .map_err(|error| map_secret_error("secret.get", error))?;
            Ok(secret.as_str().to_string())
        })
        .map_err(map_lua_error)?;

    let set_store = Arc::clone(&secret_store);
    let set_scope = secret_scope;
    let set_fn = lua
        .create_function(move |_, (key, value): (String, String)| {
            set_store
                .set(&set_scope, key.trim(), value.as_str())
                .map_err(|error| map_secret_error("secret.set", error))?;
            Ok(())
        })
        .map_err(map_lua_error)?;

    secret_table.set("get", get_fn).map_err(map_lua_error)?;
    secret_table.set("set", set_fn).map_err(map_lua_error)?;

    let globals = lua.globals();
    globals.set("secret", secret_table).map_err(map_lua_error)?;
    Ok(())
}

fn register_http_api(
    lua: &Lua,
    network_profile: &str,
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
    request_counter: Arc<AtomicUsize>,
) -> PluginRuntimeResult<()> {
    let transport_profile = NetworkProfileFactory::create(network_profile)
        .map_err(|error| PluginRuntimeError::ScriptRuntime(error.to_string()))?;
    let http_table = lua.create_table().map_err(map_lua_error)?;

    let request_cookie_store = Arc::clone(&cookie_store);
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
                Arc::clone(&request_cookie_store),
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
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
) -> Result<HttpResponseOutput, LuaError> {
    let url = Url::parse(request.url.trim())
        .map_err(|error| LuaError::runtime(format!("http.request url 非法：{error}")))?;
    ensure_allowed_target(&url)?;

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
    let headers = build_request_headers(
        transport_profile,
        request.headers.as_ref(),
        Arc::clone(&cookie_store),
    )?;

    let mut retry_attempt = 0usize;
    loop {
        if retry_attempt > 0 {
            thread::sleep(retry_backoff(
                transport_profile.request_delay(),
                retry_attempt,
            ));
        }

        let client_cloned = client.clone();
        let url_cloned = url.clone();
        let headers_cloned = headers.clone();
        let method_cloned = method.clone();
        let body = request.body.clone();

        let response = run_reqwest_blocking(async move {
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
        apply_response_cookies(&response_headers, Arc::clone(&cookie_store))?;
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

        let headers = flatten_response_headers(&response_headers);
        let body = String::from_utf8_lossy(&response_body).to_string();
        return Ok(HttpResponseOutput {
            status: status.as_u16(),
            headers,
            body,
            final_url,
        });
    }
}

fn build_request_headers(
    transport_profile: &dyn TransportProfile,
    headers: Option<&std::collections::BTreeMap<String, String>>,
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
) -> Result<HeaderMap, LuaError> {
    let mut request_headers = HeaderMap::new();
    for (name, value) in transport_profile.default_headers() {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            LuaError::runtime(format!(
                "http.request 默认 Header 名非法（{name}）：{error}"
            ))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            LuaError::runtime(format!(
                "http.request 默认 Header 值非法（{name}）：{error}"
            ))
        })?;
        request_headers.insert(header_name, header_value);
    }

    if let Some(headers) = headers {
        for (name, value) in headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                LuaError::runtime(format!("http.request Header 名非法（{name}）：{error}"))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|error| {
                LuaError::runtime(format!("http.request Header 值非法（{name}）：{error}"))
            })?;
            request_headers.insert(header_name, header_value);
        }
    }

    let has_cookie_header = request_headers.contains_key("cookie");
    if !has_cookie_header {
        let cookie_header = compose_cookie_header(cookie_store)?;
        if !cookie_header.is_empty() {
            let header_value = HeaderValue::from_str(cookie_header.as_str()).map_err(|error| {
                LuaError::runtime(format!(
                    "http.request Cookie Header 值非法（cookie）：{error}"
                ))
            })?;
            request_headers.insert(HeaderName::from_static("cookie"), header_value);
        }
    }

    Ok(request_headers)
}

fn flatten_response_headers(headers: &HeaderMap) -> std::collections::BTreeMap<String, String> {
    let mut merged = std::collections::BTreeMap::new();
    for (name, value) in headers {
        let key = name.as_str().to_string();
        let current = merged.entry(key).or_insert_with(String::new);
        if !current.is_empty() {
            current.push_str(", ");
        }
        let value = value.to_str().unwrap_or("<non-utf8>");
        current.push_str(value);
    }
    merged
}

fn compose_cookie_header(
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
) -> Result<String, LuaError> {
    let jar = cookie_store
        .lock()
        .map_err(|_| LuaError::runtime("cookie 会话锁已损坏"))?;
    if jar.is_empty() {
        return Ok(String::new());
    }

    let mut pairs = jar
        .iter()
        .map(|(name, entry)| {
            let _attrs = entry.attrs.len();
            format!("{name}={}", entry.value)
        })
        .collect::<Vec<_>>();
    pairs.sort();
    Ok(pairs.join("; "))
}

fn apply_response_cookies(
    headers: &HeaderMap,
    cookie_store: Arc<Mutex<HashMap<String, CookieEntry>>>,
) -> Result<(), LuaError> {
    let mut jar = cookie_store
        .lock()
        .map_err(|_| LuaError::runtime("cookie 会话锁已损坏"))?;
    for value in &headers.get_all("set-cookie") {
        let raw = match value.to_str() {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if let Some((name, cookie)) = parse_set_cookie_line(raw) {
            jar.insert(name, cookie);
        }
    }
    Ok(())
}

fn parse_set_cookie_line(raw: &str) -> Option<(String, CookieEntry)> {
    let mut segments = raw.split(';');
    let name_value = segments.next()?.trim();
    let (name, value) = name_value.split_once('=')?;
    if name.trim().is_empty() {
        return None;
    }

    let mut attrs = std::collections::BTreeMap::new();
    for segment in segments {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((attr_name, attr_value)) = segment.split_once('=') {
            attrs.insert(attr_name.trim().to_string(), attr_value.trim().to_string());
        } else {
            attrs.insert(segment.to_string(), "true".to_string());
        }
    }

    Some((
        name.trim().to_string(),
        CookieEntry {
            value: value.trim().to_string(),
            attrs,
        },
    ))
}

fn parse_cookie_attrs(
    attrs: Option<Table>,
) -> Result<std::collections::BTreeMap<String, String>, LuaError> {
    let mut parsed = std::collections::BTreeMap::new();
    let Some(attrs) = attrs else {
        return Ok(parsed);
    };

    for pair in attrs.pairs::<LuaValue, LuaValue>() {
        let (raw_key, raw_value) = pair?;
        let key = lua_value_to_string(raw_key)
            .ok_or_else(|| LuaError::runtime("cookie.set attrs 键必须是 string/number/boolean"))?;
        let value = lua_value_to_string(raw_value)
            .ok_or_else(|| LuaError::runtime("cookie.set attrs 值必须是 string/number/boolean"))?;
        parsed.insert(key, value);
    }

    Ok(parsed)
}

fn lua_value_to_string(value: LuaValue) -> Option<String> {
    match value {
        LuaValue::String(raw) => Some(raw.to_string_lossy().to_string()),
        LuaValue::Boolean(value) => Some(value.to_string()),
        LuaValue::Integer(value) => Some(value.to_string()),
        LuaValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_html_text(input: String) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_allowed_target(url: &Url) -> Result<(), LuaError> {
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(LuaError::runtime(format!(
                "http.request 仅支持 http/https，当前为：{scheme}"
            )));
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| LuaError::runtime("http.request 缺少 host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| LuaError::runtime("http.request 端口无效"))?;
    let addresses = resolve_host_ips(host, port)?;
    if addresses.is_empty() {
        return Err(LuaError::runtime("http.request 无法解析目标地址"));
    }

    for ip in addresses {
        if is_forbidden_ip(ip) {
            return Err(LuaError::runtime(format!(
                "http.request 目标地址不允许（内网/保留地址）：{}",
                ip
            )));
        }
    }

    Ok(())
}

fn resolve_host_ips(host: &str, port: u16) -> Result<Vec<IpAddr>, LuaError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }

    let socket_address = format!("{host}:{port}");
    socket_address
        .to_socket_addrs()
        .map(|iter| iter.map(|addr| addr.ip()).collect::<Vec<_>>())
        .map_err(|error| LuaError::runtime(format!("http.request DNS 解析失败：{error}")))
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 127
                || octets[0] == 0
                || octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let first_segment = v6.segments()[0];
            (first_segment & 0xfe00) == 0xfc00 || (first_segment & 0xffc0) == 0xfe80
        }
    }
}

fn run_reqwest_blocking<T, F>(future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
{
    let handle = thread::spawn(move || {
        let runtime = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("初始化异步运行时失败：{error}"))?;
        runtime.block_on(future)
    });

    handle
        .join()
        .map_err(|_| "HTTP 请求线程异常退出".to_string())?
}

fn retry_backoff(base_delay: Duration, retry_attempt: usize) -> Duration {
    let base_delay = if base_delay.is_zero() {
        Duration::from_millis(100)
    } else {
        base_delay
    };
    let shift = retry_attempt.saturating_sub(1).min(8);
    base_delay.saturating_mul(1_u32 << shift)
}
