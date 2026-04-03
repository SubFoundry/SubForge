use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use app_common::{AppSetting, ConfigSchema, Plugin, Profile, ProxyNode, SourceInstance};
use app_core::{Engine, PluginInstallService, SourceService};
use app_storage::{
    ExportTokenRepository, NodeCacheRepository, PluginRepository, ProfileRepository,
    RefreshJobRepository, SettingsRepository, SourceRepository,
};
use axum::Json;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::ApiEvent;
use crate::helpers::{
    config_error_response, core_error_to_response, current_timestamp_rfc3339, emit_event,
    error_response, extract_zip_to_dir, internal_error_response, list_profile_ids_by_source,
    list_profile_source_ids, load_plugin_by_route_id, map_settings, not_found_error_response,
    replace_profile_sources, source_with_config_to_dto, storage_error_to_response,
    validate_source_ids_exist, validate_zip_safety,
};
use crate::state::{
    APP_VERSION, ApiResult, HealthResponse, MAX_PLUGIN_UPLOAD_BYTES, ServerContext,
};

mod events;
mod health;
mod logs;
mod plugins;
mod profiles;
mod settings;
mod sources;

pub(crate) use events::events_handler;
pub(crate) use health::health_handler;
pub(crate) use logs::list_logs_handler;
pub(crate) use plugins::{
    delete_plugin_handler, get_plugin_schema_handler, import_plugin_handler, list_plugins_handler,
    toggle_plugin_handler,
};
pub(crate) use profiles::{
    create_profile_handler, delete_profile_handler, get_profile_base64_handler,
    get_profile_clash_handler, get_profile_raw_handler, get_profile_singbox_handler,
    list_profiles_handler, refresh_profile_handler, rotate_profile_export_token_handler,
    update_profile_handler,
};
pub(crate) use settings::{
    get_system_settings_handler, get_system_status_handler, rotate_admin_token_handler,
    shutdown_system_handler, update_system_settings_handler,
};
pub(crate) use sources::{
    create_source_handler, delete_source_handler, list_sources_handler, refresh_source_handler,
    update_source_handler,
};

#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    pub(crate) settings: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SystemStatusResponse {
    pub(crate) active_sources: usize,
    pub(crate) total_nodes: usize,
    pub(crate) last_refresh_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShutdownResponse {
    pub(crate) accepted: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct RotateAdminTokenResponse {
    pub(crate) token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSettingsRequest {
    pub(crate) settings: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PluginListResponse {
    pub(crate) plugins: Vec<Plugin>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TogglePluginRequest {
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SourceDto {
    pub(crate) source: SourceInstance,
    pub(crate) config: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceListResponse {
    pub(crate) sources: Vec<SourceDto>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceResponse {
    pub(crate) source: SourceDto,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSourceRequest {
    pub(crate) plugin_id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) config: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSourceRequest {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) config: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshSourceResponse {
    pub(crate) source_id: String,
    pub(crate) node_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshProfileResponse {
    pub(crate) profile_id: String,
    pub(crate) refreshed_sources: usize,
    pub(crate) node_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct RotateProfileExportTokenResponse {
    pub(crate) profile_id: String,
    pub(crate) token: String,
    pub(crate) previous_token_expires_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LogsQuery {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    #[serde(default)]
    pub(crate) offset: Option<usize>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) source_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshLogDto {
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) source_name: Option<String>,
    pub(crate) trigger_type: String,
    pub(crate) status: String,
    pub(crate) started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    pub(crate) node_count: Option<i64>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LogsResponse {
    pub(crate) logs: Vec<RefreshLogDto>,
    pub(crate) pagination: LogsPagination,
}

#[derive(Debug, Serialize)]
pub(crate) struct LogsPagination {
    pub(crate) limit: usize,
    pub(crate) offset: usize,
    pub(crate) total: usize,
    pub(crate) has_more: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct PluginSchemaResponse {
    pub(crate) plugin_id: String,
    pub(crate) name: String,
    pub(crate) plugin_type: String,
    pub(crate) secret_fields: Vec<String>,
    pub(crate) schema: ConfigSchema,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProfileDto {
    pub(crate) profile: Profile,
    pub(crate) source_ids: Vec<String>,
    pub(crate) export_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileListResponse {
    pub(crate) profiles: Vec<ProfileDto>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileResponse {
    pub(crate) profile: ProfileDto,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateProfileRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateProfileRequest {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<Option<String>>,
    #[serde(default)]
    pub(crate) source_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenQuery {
    #[serde(default)]
    pub(crate) token: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileRawResponse {
    pub(crate) profile_id: String,
    pub(crate) profile_name: String,
    pub(crate) node_count: usize,
    pub(crate) generated_at: String,
    pub(crate) nodes: Vec<ProxyNode>,
}

pub(crate) type ApiResponseResult = Result<Response, (StatusCode, Json<app_common::ErrorResponse>)>;
