use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use app_common::{AppSetting, Plugin, Profile, ProxyNode, SourceInstance};
use app_core::{Engine, PluginInstallService, SourceService};
use app_storage::{
    NodeCacheRepository, PluginRepository, ProfileRepository, SettingsRepository, SourceRepository,
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
mod plugins;
mod profiles;
mod settings;
mod sources;

pub(crate) use events::events_handler;
pub(crate) use health::health_handler;
pub(crate) use plugins::{delete_plugin_handler, import_plugin_handler, list_plugins_handler};
pub(crate) use profiles::{
    create_profile_handler, delete_profile_handler, get_profile_base64_handler,
    get_profile_clash_handler, get_profile_raw_handler, get_profile_singbox_handler,
    list_profiles_handler, refresh_profile_handler, update_profile_handler,
};
pub(crate) use settings::{get_system_settings_handler, update_system_settings_handler};
pub(crate) use sources::{
    create_source_handler, delete_source_handler, list_sources_handler, refresh_source_handler,
    update_source_handler,
};

#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    pub(crate) settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSettingsRequest {
    pub(crate) settings: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PluginListResponse {
    pub(crate) plugins: Vec<Plugin>,
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProfileDto {
    pub(crate) profile: Profile,
    pub(crate) source_ids: Vec<String>,
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
