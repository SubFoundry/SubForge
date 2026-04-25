use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use mlua::{Lua, Table, Value as LuaValue};
use serde::{Deserialize, Serialize};

use super::error_map::{map_lua_error, map_secret_error};
use super::{
    HTTP_REQUEST_LIMIT_SENTINEL, HTTP_RESPONSE_LIMIT_SENTINEL, LOG_PREFIX, LuaSandboxConfig,
    RuntimeLogSink, SCRIPT_HTTP_MAX_REDIRECTS, SCRIPT_HTTP_MAX_REQUESTS,
    SCRIPT_HTTP_MAX_RESPONSE_BYTES, SCRIPT_HTTP_TIMEOUT_MS,
};
use crate::PluginRuntimeResult;

mod base64_api;
mod cookie_api;
mod html_api;
mod http_api;
mod json_api;
mod log_api;
mod secret_api;
mod time_api;

pub(super) type CookieStore = Arc<Mutex<HashMap<String, CookieEntry>>>;

#[derive(Debug, Deserialize)]
pub(super) struct HttpRequestInput {
    pub(super) url: String,
    pub(super) method: Option<String>,
    pub(super) headers: Option<std::collections::BTreeMap<String, String>>,
    pub(super) body: Option<String>,
    pub(super) timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct CookieEntry {
    pub(super) value: String,
    pub(super) attrs: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(super) struct HttpResponseOutput {
    pub(super) status: u16,
    pub(super) headers: std::collections::BTreeMap<String, String>,
    pub(super) body: String,
    pub(super) final_url: String,
}

pub(super) fn new_cookie_store() -> CookieStore {
    Arc::new(Mutex::new(HashMap::new()))
}

pub(super) fn register_runtime_apis(
    lua: &Lua,
    config: &LuaSandboxConfig,
    cookie_store: CookieStore,
    secret_scope: String,
    request_counter: Arc<AtomicUsize>,
    log_sink: Option<Arc<dyn RuntimeLogSink>>,
) -> PluginRuntimeResult<()> {
    let has_capability = |capability: &str| {
        config
            .capabilities
            .iter()
            .any(|enabled| enabled.as_str() == capability)
    };

    if has_capability("json") {
        json_api::register_json_api(lua)?;
    }
    if has_capability("base64") {
        base64_api::register_base64_api(lua)?;
    }
    if has_capability("time") {
        time_api::register_time_api(lua)?;
    }
    if has_capability("log") {
        log_api::register_log_api(lua, log_sink)?;
    }
    if has_capability("html") {
        html_api::register_html_api(lua)?;
    }
    if has_capability("cookie") {
        cookie_api::register_cookie_api(lua, Arc::clone(&cookie_store))?;
    }
    if has_capability("secret") {
        secret_api::register_secret_api(lua, Arc::clone(&config.secret_store), secret_scope)?;
    }
    if has_capability("http") {
        http_api::register_http_api(lua, &config.network_profile, cookie_store, request_counter)?;
    }
    Ok(())
}

pub(super) fn parse_cookie_attrs(
    attrs: Option<Table>,
) -> Result<std::collections::BTreeMap<String, String>, mlua::Error> {
    let mut parsed = std::collections::BTreeMap::new();
    let Some(attrs) = attrs else {
        return Ok(parsed);
    };

    for pair in attrs.pairs::<LuaValue, LuaValue>() {
        let (raw_key, raw_value) = pair?;
        let key = lua_value_to_string(raw_key).ok_or_else(|| {
            mlua::Error::runtime("cookie.set attrs 键必须是 string/number/boolean")
        })?;
        let value = lua_value_to_string(raw_value).ok_or_else(|| {
            mlua::Error::runtime("cookie.set attrs 值必须是 string/number/boolean")
        })?;
        parsed.insert(key, value);
    }

    Ok(parsed)
}

pub(super) fn lua_value_to_string(value: LuaValue) -> Option<String> {
    match value {
        LuaValue::String(raw) => Some(raw.to_string_lossy().to_string()),
        LuaValue::Boolean(value) => Some(value.to_string()),
        LuaValue::Integer(value) => Some(value.to_string()),
        LuaValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
