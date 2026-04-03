use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use super::super::CookieStore;
use super::cookies::compose_cookie_header;
use app_transport::TransportProfile;
use mlua::Error as LuaError;

pub(super) fn build_request_headers(
    transport_profile: &dyn TransportProfile,
    headers: Option<&std::collections::BTreeMap<String, String>>,
    cookie_store: CookieStore,
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

pub(super) fn flatten_response_headers(
    headers: &HeaderMap,
) -> std::collections::BTreeMap<String, String> {
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
