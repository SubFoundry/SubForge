use std::time::Duration;
use std::{net::SocketAddr, str::FromStr};

use axum::body::Body;
use axum::extract::State;
use axum::extract::connect_info::ConnectInfo;
use axum::http::{
    Method, Request, StatusCode, header::AUTHORIZATION, header::FORWARDED, header::HOST,
    header::USER_AGENT,
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::helpers::{
    error_response, extract_profile_id_from_path, extract_query_param, internal_error_response,
    is_profile_read_endpoint, is_valid_export_token, normalize_host, parse_bearer_token,
    unauthorized_error_response,
};
use crate::state::{
    MANAGEMENT_RATE_LIMIT_PER_SECOND, SUBSCRIPTION_RATE_LIMIT_PER_SECOND, ServerContext,
};
pub(crate) async fn host_validation_middleware(
    State(state): State<ServerContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_host)
        .unwrap_or_default();

    if !state.host_validation.is_allowed(&host) {
        return error_response(
            StatusCode::FORBIDDEN,
            "E_AUTH",
            "Forbidden: invalid Host header",
            false,
        )
        .into_response();
    }
    next.run(request).await
}

pub(crate) async fn cors_reject_middleware(
    State(_state): State<ServerContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    next.run(request).await
}

pub(crate) async fn rate_limit_middleware(
    State(state): State<ServerContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if path == "/health" || request.method() == Method::OPTIONS {
        return next.run(request).await;
    }

    let source_key = request_source_key(&request);
    let (key, limit) = if is_profile_read_endpoint(request.method(), &path) {
        let token = extract_query_param(request.uri().query(), "token")
            .unwrap_or_else(|| "__missing__".to_string());
        (
            format!("profile-read:{token}"),
            SUBSCRIPTION_RATE_LIMIT_PER_SECOND,
        )
    } else if path.starts_with("/api/") {
        (
            format!("management:{source_key}"),
            MANAGEMENT_RATE_LIMIT_PER_SECOND,
        )
    } else {
        return next.run(request).await;
    };

    if !state
        .rate_limiter
        .is_allowed(&key, limit, Duration::from_secs(1))
    {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "E_RATE_LIMIT",
            "Too many requests",
            true,
        )
        .into_response();
    }

    next.run(request).await
}

pub(crate) async fn admin_auth_middleware(
    State(state): State<ServerContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if path == "/health" || request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    if !path.starts_with("/api/") {
        return next.run(request).await;
    }
    let source_key = request_source_key(&request);

    if state.auth_failures.is_in_cooldown(&source_key) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "E_RATE_LIMIT",
            "Too many authentication failures, please retry later",
            true,
        )
        .into_response();
    }

    let admin_ok = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_bearer_token)
        .is_some_and(|token| {
            state
                .admin_token
                .read()
                .is_ok_and(|current| token == current.as_str())
        });
    if admin_ok {
        state.auth_failures.reset(&source_key);
        return next.run(request).await;
    }

    if is_profile_read_endpoint(request.method(), &path)
        && let Some(profile_id) = extract_profile_id_from_path(&path)
        && let Some(token) = extract_query_param(request.uri().query(), "token")
    {
        match is_valid_export_token(state.database.as_ref(), profile_id, &token) {
            Ok(true) => {
                state.auth_failures.reset(&source_key);
                return next.run(request).await;
            }
            Ok(false) => {}
            Err(_) => return internal_error_response().into_response(),
        }
    }

    if state.auth_failures.record_failure(&source_key) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "E_RATE_LIMIT",
            "Too many authentication failures, please retry later",
            true,
        )
        .into_response();
    }

    unauthorized_error_response().into_response()
}

fn request_source_key(request: &Request<Body>) -> String {
    if let Some(client_id) = header_value(request, "x-subforge-client-id") {
        return format!("client:{}", normalize_source_value(client_id));
    }
    if let Some(ip) =
        header_value(request, "x-forwarded-for").and_then(|value| value.split(',').next())
    {
        return format!("xff:{}", normalize_source_value(ip));
    }
    if let Some(ip) = header_value(request, "x-real-ip") {
        return format!("xri:{}", normalize_source_value(ip));
    }
    if let Some(forwarded) = request
        .headers()
        .get(FORWARDED)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_forwarded_for)
    {
        return format!("fwd:{}", normalize_source_value(&forwarded));
    }
    if let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return format!("peer:{peer}");
    }
    if let Some(agent) = request
        .headers()
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
    {
        return format!("ua:{}", normalize_source_value(agent));
    }
    "unknown".to_string()
}

fn header_value<'a>(request: &'a Request<Body>, header_name: &str) -> Option<&'a str> {
    request
        .headers()
        .get(header_name)
        .and_then(|value| value.to_str().ok())
}

fn parse_forwarded_for(raw: &str) -> Option<String> {
    raw.split(',').find_map(|entry| {
        entry.split(';').find_map(|segment| {
            let trimmed = segment.trim();
            let value = trimmed.strip_prefix("for=")?;
            let value = value.trim_matches('"').trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        })
    })
}

fn normalize_source_value(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return "unknown".to_string();
    }
    if let Ok(address) = SocketAddr::from_str(value) {
        return address.ip().to_string();
    }
    value.chars().take(128).collect()
}
