use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header::AUTHORIZATION, header::HOST};
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

    let (key, limit) = if is_profile_read_endpoint(request.method(), &path) {
        let token = extract_query_param(request.uri().query(), "token")
            .unwrap_or_else(|| "__missing__".to_string());
        (
            format!("profile-read:{token}"),
            SUBSCRIPTION_RATE_LIMIT_PER_SECOND,
        )
    } else if path.starts_with("/api/") {
        ("management".to_string(), MANAGEMENT_RATE_LIMIT_PER_SECOND)
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

    if state.auth_failures.is_in_cooldown() {
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
        state.auth_failures.reset();
        return next.run(request).await;
    }

    if is_profile_read_endpoint(request.method(), &path)
        && let Some(profile_id) = extract_profile_id_from_path(&path)
        && let Some(token) = extract_query_param(request.uri().query(), "token")
    {
        match is_valid_export_token(state.database.as_ref(), profile_id, &token) {
            Ok(true) => {
                state.auth_failures.reset();
                return next.run(request).await;
            }
            Ok(false) => {}
            Err(_) => return internal_error_response().into_response(),
        }
    }

    if state.auth_failures.record_failure() {
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
