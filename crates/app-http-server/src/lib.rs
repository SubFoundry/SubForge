//! app-http-server：HTTP API 路由与中间件封装。

mod handlers;
mod helpers;
mod middleware;
mod state;

#[cfg(test)]
mod tests;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::{self as axum_middleware};
use axum::routing::{delete, get, post, put};

use handlers::{
    create_profile_handler, create_source_handler, delete_plugin_handler, delete_profile_handler,
    delete_source_handler, events_handler, get_profile_base64_handler, get_profile_clash_handler,
    get_profile_raw_handler, get_profile_singbox_handler, get_system_settings_handler,
    get_system_status_handler, health_handler, import_plugin_handler, list_logs_handler,
    list_plugins_handler, list_profiles_handler, list_sources_handler, refresh_profile_handler,
    refresh_source_handler, toggle_plugin_handler, update_profile_handler, update_source_handler,
    update_system_settings_handler,
};
use middleware::{
    admin_auth_middleware, cors_reject_middleware, host_validation_middleware,
    rate_limit_middleware,
};
use state::MAX_PLUGIN_UPLOAD_BYTES;

pub use state::{ApiEvent, ServerContext};

pub fn build_router(state: ServerContext) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route(
            "/api/system/settings",
            get(get_system_settings_handler).put(update_system_settings_handler),
        )
        .route("/api/system/status", get(get_system_status_handler))
        .route("/api/logs", get(list_logs_handler))
        .route("/api/plugins", get(list_plugins_handler))
        .route("/api/plugins/import", post(import_plugin_handler))
        .route("/api/plugins/{id}", delete(delete_plugin_handler))
        .route("/api/plugins/{id}/toggle", put(toggle_plugin_handler))
        .route(
            "/api/sources",
            get(list_sources_handler).post(create_source_handler),
        )
        .route(
            "/api/sources/{id}",
            put(update_source_handler).delete(delete_source_handler),
        )
        .route("/api/sources/{id}/refresh", post(refresh_source_handler))
        .route(
            "/api/profiles",
            get(list_profiles_handler).post(create_profile_handler),
        )
        .route(
            "/api/profiles/{id}",
            put(update_profile_handler).delete(delete_profile_handler),
        )
        .route("/api/profiles/{id}/refresh", post(refresh_profile_handler))
        .route("/api/profiles/{id}/clash", get(get_profile_clash_handler))
        .route(
            "/api/profiles/{id}/sing-box",
            get(get_profile_singbox_handler),
        )
        .route("/api/profiles/{id}/base64", get(get_profile_base64_handler))
        .route("/api/profiles/{id}/raw", get(get_profile_raw_handler))
        .route("/api/events", get(events_handler))
        .layer(DefaultBodyLimit::max(MAX_PLUGIN_UPLOAD_BYTES))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            admin_auth_middleware,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            cors_reject_middleware,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            host_validation_middleware,
        ))
        .with_state(state)
}
