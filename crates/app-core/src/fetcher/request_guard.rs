use reqwest::Url;
use reqwest::header::{CONTENT_TYPE, HeaderMap};

use crate::{CoreError, CoreResult};

pub(crate) fn redact_url_for_log(url: &Url) -> String {
    let mut sanitized = url.clone();
    if url.query().is_none() {
        return sanitized.to_string();
    }
    sanitized.set_query(None);
    {
        let mut query = sanitized.query_pairs_mut();
        for (key, value) in url.query_pairs() {
            if is_sensitive_query_key(key.as_ref()) {
                query.append_pair(key.as_ref(), "***");
            } else {
                query.append_pair(key.as_ref(), value.as_ref());
            }
        }
    }
    sanitized.to_string()
}

pub(crate) fn sanitize_reqwest_error(error: &reqwest::Error, url: &Url) -> String {
    let message = error.to_string();
    let redacted_url = redact_url_for_log(url);
    message.replace(url.as_str(), &redacted_url)
}

pub(crate) fn redact_headers_for_log(headers: &HeaderMap) -> String {
    if headers.is_empty() {
        return "[]".to_string();
    }
    let mut pairs = headers
        .iter()
        .map(|(name, value)| {
            let key = name.as_str().to_ascii_lowercase();
            let value = if is_sensitive_header(&key) {
                "***".to_string()
            } else {
                value.to_str().unwrap_or("<non-utf8>").to_string()
            };
            format!("{key}={value}")
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    format!("[{}]", pairs.join(", "))
}

pub(crate) fn is_sensitive_query_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "token"
            | "access_token"
            | "password"
            | "passwd"
            | "secret"
            | "auth"
            | "authorization"
            | "api_key"
            | "apikey"
            | "cookie"
    )
}

pub(crate) fn is_sensitive_header(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
    )
}

pub(crate) fn validate_content_type(headers: &HeaderMap) -> CoreResult<()> {
    let Some(content_type) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };

    let normalized = content_type.to_ascii_lowercase();
    let allowed = normalized.starts_with("text/")
        || normalized.starts_with("application/json")
        || normalized.starts_with("application/octet-stream");

    if allowed {
        Ok(())
    } else {
        Err(CoreError::SubscriptionFetch(format!(
            "上游 Content-Type 不受支持：{content_type}"
        )))
    }
}
