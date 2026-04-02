//! app-http-server：HTTP API 路由与中间件封装。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_common::{AppSetting, ErrorResponse, Plugin, Profile, ProxyNode, SourceInstance};
use app_core::{CoreError, Engine, PluginInstallService, SourceService, SourceWithConfig};
use app_secrets::SecretStore;
use app_storage::{
    Database, ExportTokenRepository, NodeCacheRepository, PluginRepository, ProfileRepository,
    SettingsRepository, SourceRepository, StorageError,
};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::{Method, Request, StatusCode, header::AUTHORIZATION, header::HOST};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use zip::ZipArchive;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_PLUGIN_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 100;
const MAX_ZIP_TOTAL_UNCOMPRESSED_BYTES: u64 = 50 * 1024 * 1024;
const AUTH_FAILURE_THRESHOLD: u32 = 5;
const AUTH_FAILURE_COOLDOWN_SECONDS: u64 = 60;
const MANAGEMENT_RATE_LIMIT_PER_SECOND: u32 = 30;
const SUBSCRIPTION_RATE_LIMIT_PER_SECOND: u32 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEvent {
    pub event: String,
    pub message: String,
    pub source_id: Option<String>,
    pub timestamp: String,
}

#[derive(Clone)]
pub struct ServerContext {
    admin_token: Arc<String>,
    database: Arc<Database>,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: PathBuf,
    host_validation: HostValidationState,
    event_sender: broadcast::Sender<ApiEvent>,
    rate_limiter: Arc<RateLimiter>,
    auth_failures: Arc<AuthFailures>,
}

impl ServerContext {
    pub fn new(
        admin_token: String,
        database: Arc<Database>,
        secret_store: Arc<dyn SecretStore>,
        plugins_dir: PathBuf,
        listen_port: u16,
        event_sender: broadcast::Sender<ApiEvent>,
    ) -> Self {
        Self {
            admin_token: Arc::new(admin_token),
            database,
            secret_store,
            plugins_dir,
            host_validation: HostValidationState::new(listen_port),
            event_sender,
            rate_limiter: Arc::new(RateLimiter::default()),
            auth_failures: Arc::new(AuthFailures::default()),
        }
    }
}

#[derive(Debug, Clone)]
struct HostValidationState {
    allowed_hosts: Arc<HashSet<String>>,
}

impl HostValidationState {
    fn new(port: u16) -> Self {
        let mut hosts = HashSet::new();
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            hosts.insert(host.to_string());
            hosts.insert(format!("{host}:{port}"));
        }
        Self {
            allowed_hosts: Arc::new(hosts),
        }
    }

    fn is_allowed(&self, host_header: &str) -> bool {
        self.allowed_hosts.contains(host_header)
    }
}

#[derive(Debug, Default)]
struct RateLimiter {
    windows: Mutex<HashMap<String, RateWindow>>,
}

#[derive(Debug, Clone, Copy)]
struct RateWindow {
    started_at: Instant,
    count: u32,
}

impl RateLimiter {
    fn is_allowed(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut windows = match self.windows.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let now = Instant::now();
        let entry = windows.entry(key.to_string()).or_insert(RateWindow {
            started_at: now,
            count: 0,
        });
        if now.duration_since(entry.started_at) >= window {
            entry.started_at = now;
            entry.count = 0;
        }
        if entry.count >= limit {
            return false;
        }
        entry.count += 1;
        true
    }
}

#[derive(Debug, Default)]
struct AuthFailures {
    inner: Mutex<AuthFailuresState>,
}

#[derive(Debug, Default)]
struct AuthFailuresState {
    failures: u32,
    cooldown_until: Option<Instant>,
}

impl AuthFailures {
    fn is_in_cooldown(&self) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return true,
        };
        if let Some(deadline) = inner.cooldown_until {
            if Instant::now() < deadline {
                return true;
            }
            inner.cooldown_until = None;
            inner.failures = 0;
        }
        false
    }

    fn record_failure(&self) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return true,
        };
        inner.failures += 1;
        if inner.failures >= AUTH_FAILURE_THRESHOLD {
            inner.failures = 0;
            inner.cooldown_until =
                Some(Instant::now() + Duration::from_secs(AUTH_FAILURE_COOLDOWN_SECONDS));
            return true;
        }
        false
    }

    fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.failures = 0;
            inner.cooldown_until = None;
        }
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

type ApiResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<ErrorResponse>)>;

pub fn build_router(state: ServerContext) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route(
            "/api/system/settings",
            get(get_system_settings_handler).put(update_system_settings_handler),
        )
        .route("/api/plugins", get(list_plugins_handler))
        .route("/api/plugins/import", post(import_plugin_handler))
        .route("/api/plugins/{id}", delete(delete_plugin_handler))
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
        .route("/api/profiles/{id}/raw", get(get_profile_raw_handler))
        .route("/api/events", get(events_handler))
        .layer(DefaultBodyLimit::max(MAX_PLUGIN_UPLOAD_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cors_reject_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_validation_middleware,
        ))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSettingsRequest {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct PluginListResponse {
    plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Serialize)]
struct SourceDto {
    source: SourceInstance,
    config: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
struct SourceListResponse {
    sources: Vec<SourceDto>,
}

#[derive(Debug, Serialize)]
struct SourceResponse {
    source: SourceDto,
}

#[derive(Debug, Deserialize)]
struct CreateSourceRequest {
    plugin_id: String,
    name: String,
    #[serde(default)]
    config: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct UpdateSourceRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    config: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Serialize)]
struct RefreshSourceResponse {
    source_id: String,
    node_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileDto {
    profile: Profile,
    source_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProfileListResponse {
    profiles: Vec<ProfileDto>,
}

#[derive(Debug, Serialize)]
struct ProfileResponse {
    profile: ProfileDto,
}

#[derive(Debug, Deserialize)]
struct CreateProfileRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateProfileRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<Option<String>>,
    #[serde(default)]
    source_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProfileRawResponse {
    profile_id: String,
    profile_name: String,
    node_count: usize,
    generated_at: String,
    nodes: Vec<ProxyNode>,
}

async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            version: APP_VERSION,
        }),
    )
}

async fn get_system_settings_handler(
    State(state): State<ServerContext>,
) -> ApiResult<SettingsResponse> {
    let repository = SettingsRepository::new(state.database.as_ref());
    let settings = repository.get_all().map_err(storage_error_to_response)?;
    Ok((
        StatusCode::OK,
        Json(SettingsResponse {
            settings: map_settings(settings),
        }),
    ))
}

async fn update_system_settings_handler(
    State(state): State<ServerContext>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> ApiResult<SettingsResponse> {
    if payload.settings.is_empty() {
        return Err(config_error_response("请求体 settings 不能为空"));
    }
    let updated_at = current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
    let repository = SettingsRepository::new(state.database.as_ref());
    for (key, value) in payload.settings {
        if key.trim().is_empty() {
            return Err(config_error_response("设置键不能为空"));
        }
        repository
            .set(&AppSetting {
                key,
                value,
                updated_at: updated_at.clone(),
            })
            .map_err(storage_error_to_response)?;
    }

    let settings = repository.get_all().map_err(storage_error_to_response)?;
    Ok((
        StatusCode::OK,
        Json(SettingsResponse {
            settings: map_settings(settings),
        }),
    ))
}

async fn list_plugins_handler(State(state): State<ServerContext>) -> ApiResult<PluginListResponse> {
    let repository = PluginRepository::new(state.database.as_ref());
    let plugins = repository.list().map_err(storage_error_to_response)?;
    Ok((StatusCode::OK, Json(PluginListResponse { plugins })))
}

async fn import_plugin_handler(
    State(state): State<ServerContext>,
    mut multipart: Multipart,
) -> ApiResult<Plugin> {
    let mut payload: Option<Vec<u8>> = None;
    let mut name: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| config_error_response("multipart 解析失败"))?
    {
        let Some(file_name) = field.file_name().map(str::to_string) else {
            continue;
        };
        if !file_name.to_ascii_lowercase().ends_with(".zip") {
            return Err(config_error_response("仅支持 .zip 插件包"));
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|_| config_error_response("读取上传文件失败"))?;
        if bytes.len() > MAX_PLUGIN_UPLOAD_BYTES {
            return Err(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "E_PLUGIN_INVALID",
                format!("插件包超过 {} bytes 限制", MAX_PLUGIN_UPLOAD_BYTES),
                false,
            ));
        }
        payload = Some(bytes.to_vec());
        name = Some(file_name);
        break;
    }

    let payload = payload.ok_or_else(|| config_error_response("未找到插件 zip 文件"))?;
    validate_zip_safety(&payload)?;
    let temp_dir = tempfile::tempdir().map_err(|_| internal_error_response())?;
    extract_zip_to_dir(&payload, temp_dir.path())?;

    if !temp_dir.path().join("plugin.json").exists() {
        return Err(config_error_response("插件包中缺少 plugin.json"));
    }

    let service = PluginInstallService::new(state.database.as_ref(), &state.plugins_dir);
    let installed = service
        .install_from_dir(temp_dir.path())
        .map_err(core_error_to_response)?;

    emit_event(
        &state,
        "plugin:imported",
        format!(
            "插件导入成功：{} ({})",
            installed.plugin_id,
            name.unwrap_or_else(|| "plugin.zip".to_string())
        ),
        None,
    );

    Ok((StatusCode::CREATED, Json(installed)))
}

async fn delete_plugin_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Plugin> {
    let repository = PluginRepository::new(state.database.as_ref());
    let plugin = load_plugin_by_route_id(&repository, &id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("插件不存在"))?;

    let source_repository = SourceRepository::new(state.database.as_ref());
    let attached_sources = source_repository
        .list_by_plugin(&plugin.plugin_id)
        .map_err(storage_error_to_response)?;
    if !attached_sources.is_empty() {
        return Err(error_response(
            StatusCode::CONFLICT,
            "E_CONFIG_INVALID",
            "插件仍有关联来源实例，禁止删除",
            false,
        ));
    }

    repository
        .delete(&plugin.id)
        .map_err(storage_error_to_response)?;
    let plugin_dir = state.plugins_dir.join(&plugin.plugin_id);
    if plugin_dir.exists() {
        fs::remove_dir_all(&plugin_dir).map_err(|_| internal_error_response())?;
    }

    emit_event(
        &state,
        "plugin:removed",
        format!("插件已删除：{}", plugin.plugin_id),
        None,
    );
    Ok((StatusCode::OK, Json(plugin)))
}

async fn list_sources_handler(State(state): State<ServerContext>) -> ApiResult<SourceListResponse> {
    let service = SourceService::new(
        state.database.as_ref(),
        &state.plugins_dir,
        state.secret_store.as_ref(),
    );
    let sources = service
        .list_sources()
        .map_err(core_error_to_response)?
        .into_iter()
        .map(source_with_config_to_dto)
        .collect();
    Ok((StatusCode::OK, Json(SourceListResponse { sources })))
}

async fn create_source_handler(
    State(state): State<ServerContext>,
    Json(payload): Json<CreateSourceRequest>,
) -> ApiResult<SourceResponse> {
    let service = SourceService::new(
        state.database.as_ref(),
        &state.plugins_dir,
        state.secret_store.as_ref(),
    );
    let source = service
        .create_source(&payload.plugin_id, &payload.name, payload.config)
        .map_err(core_error_to_response)?;

    emit_event(
        &state,
        "source:created",
        format!("来源创建成功：{}", source.source.id),
        Some(source.source.id.clone()),
    );
    Ok((
        StatusCode::CREATED,
        Json(SourceResponse {
            source: source_with_config_to_dto(source),
        }),
    ))
}

async fn update_source_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<UpdateSourceRequest>,
) -> ApiResult<SourceResponse> {
    if payload.name.as_deref().map(str::trim).is_none() && payload.config.is_none() {
        return Err(config_error_response("至少提供 name 或 config 之一"));
    }

    let service = SourceService::new(
        state.database.as_ref(),
        &state.plugins_dir,
        state.secret_store.as_ref(),
    );

    let mut source = if let Some(config) = payload.config {
        service
            .update_source_config(&id, config)
            .map_err(core_error_to_response)?
    } else {
        service
            .get_source(&id)
            .map_err(core_error_to_response)?
            .ok_or_else(|| not_found_error_response("来源不存在"))?
    };

    if let Some(name) = payload.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(config_error_response("name 不能为空"));
        }
        source.source.name = name.to_string();
        source.source.updated_at =
            current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
        let source_repository = SourceRepository::new(state.database.as_ref());
        source_repository
            .update(&source.source)
            .map_err(storage_error_to_response)?;
    }

    emit_event(
        &state,
        "source:updated",
        format!("来源更新成功：{id}"),
        Some(id.clone()),
    );
    Ok((
        StatusCode::OK,
        Json(SourceResponse {
            source: source_with_config_to_dto(source),
        }),
    ))
}

async fn delete_source_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    let service = SourceService::new(
        state.database.as_ref(),
        &state.plugins_dir,
        state.secret_store.as_ref(),
    );
    service.delete_source(&id).map_err(core_error_to_response)?;
    emit_event(
        &state,
        "source:deleted",
        format!("来源已删除：{id}"),
        Some(id.clone()),
    );
    Ok((StatusCode::OK, Json(json!({ "deleted": true, "id": id }))))
}

async fn refresh_source_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<RefreshSourceResponse> {
    let engine = Engine::new(
        state.database.as_ref(),
        &state.plugins_dir,
        Arc::clone(&state.secret_store),
    );
    let result = engine.refresh_source(&id, "manual").await;

    match result {
        Ok(refresh_result) => {
            emit_event(
                &state,
                "refresh:complete",
                format!("来源刷新成功：{id}，节点 {} 条", refresh_result.node_count),
                Some(id.clone()),
            );
            Ok((
                StatusCode::OK,
                Json(RefreshSourceResponse {
                    source_id: id,
                    node_count: refresh_result.node_count,
                }),
            ))
        }
        Err(error) => {
            emit_event(
                &state,
                "refresh:failed",
                format!("来源刷新失败：{id}，{error}"),
                Some(id),
            );
            Err(core_error_to_response(error))
        }
    }
}

async fn list_profiles_handler(
    State(state): State<ServerContext>,
) -> ApiResult<ProfileListResponse> {
    let repository = ProfileRepository::new(state.database.as_ref());
    let profiles = repository.list().map_err(storage_error_to_response)?;
    let mut items = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let source_ids = list_profile_source_ids(state.database.as_ref(), &profile.id)
            .map_err(storage_error_to_response)?;
        items.push(ProfileDto {
            profile,
            source_ids,
        });
    }
    Ok((
        StatusCode::OK,
        Json(ProfileListResponse { profiles: items }),
    ))
}

async fn create_profile_handler(
    State(state): State<ServerContext>,
    Json(payload): Json<CreateProfileRequest>,
) -> ApiResult<ProfileResponse> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(config_error_response("profile.name 不能为空"));
    }
    validate_source_ids_exist(state.database.as_ref(), &payload.source_ids)?;

    let now = current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
    let profile = Profile {
        id: format!(
            "profile-{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ),
        name: name.to_string(),
        description: payload.description.map(|value| value.trim().to_string()),
        created_at: now.clone(),
        updated_at: now,
    };
    let repository = ProfileRepository::new(state.database.as_ref());
    repository
        .insert(&profile)
        .map_err(storage_error_to_response)?;
    replace_profile_sources(state.database.as_ref(), &profile.id, &payload.source_ids)
        .map_err(storage_error_to_response)?;
    let engine = Engine::new(
        state.database.as_ref(),
        &state.plugins_dir,
        Arc::clone(&state.secret_store),
    );
    if let Err(error) = engine.ensure_profile_export_token(&profile.id) {
        let _ = repository.delete(&profile.id);
        return Err(core_error_to_response(error));
    }

    emit_event(
        &state,
        "profile:created",
        format!("Profile 创建成功：{}", profile.id),
        None,
    );
    Ok((
        StatusCode::CREATED,
        Json(ProfileResponse {
            profile: ProfileDto {
                profile,
                source_ids: payload.source_ids,
            },
        }),
    ))
}

async fn update_profile_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<UpdateProfileRequest>,
) -> ApiResult<ProfileResponse> {
    let repository = ProfileRepository::new(state.database.as_ref());
    let mut profile = repository
        .get_by_id(&id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("Profile 不存在"))?;

    if let Some(name) = payload.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(config_error_response("profile.name 不能为空"));
        }
        profile.name = name.to_string();
    }
    if let Some(description) = payload.description {
        profile.description = description.map(|value| value.trim().to_string());
    }
    profile.updated_at = current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
    repository
        .update(&profile)
        .map_err(storage_error_to_response)?;

    let source_ids = if let Some(source_ids) = payload.source_ids {
        validate_source_ids_exist(state.database.as_ref(), &source_ids)?;
        replace_profile_sources(state.database.as_ref(), &id, &source_ids)
            .map_err(storage_error_to_response)?;
        source_ids
    } else {
        list_profile_source_ids(state.database.as_ref(), &id).map_err(storage_error_to_response)?
    };

    emit_event(
        &state,
        "profile:updated",
        format!("Profile 更新成功：{id}"),
        None,
    );
    Ok((
        StatusCode::OK,
        Json(ProfileResponse {
            profile: ProfileDto {
                profile,
                source_ids,
            },
        }),
    ))
}

async fn delete_profile_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    let repository = ProfileRepository::new(state.database.as_ref());
    let affected = repository.delete(&id).map_err(storage_error_to_response)?;
    if affected == 0 {
        return Err(not_found_error_response("Profile 不存在"));
    }
    emit_event(
        &state,
        "profile:deleted",
        format!("Profile 已删除：{id}"),
        None,
    );
    Ok((StatusCode::OK, Json(json!({ "deleted": true, "id": id }))))
}

async fn get_profile_raw_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenQuery>,
) -> ApiResult<ProfileRawResponse> {
    let _ = query.token.as_deref();
    let profile_repository = ProfileRepository::new(state.database.as_ref());
    let profile = profile_repository
        .get_by_id(&id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("Profile 不存在"))?;

    let source_ids =
        list_profile_source_ids(state.database.as_ref(), &id).map_err(storage_error_to_response)?;
    let cache_repository = NodeCacheRepository::new(state.database.as_ref());
    let mut nodes = Vec::new();
    for source_id in source_ids {
        if let Some(entry) = cache_repository
            .get_by_source(&source_id)
            .map_err(storage_error_to_response)?
        {
            nodes.extend(entry.nodes);
        }
    }

    Ok((
        StatusCode::OK,
        Json(ProfileRawResponse {
            profile_id: profile.id,
            profile_name: profile.name,
            node_count: nodes.len(),
            generated_at: current_timestamp_rfc3339().map_err(|_| internal_error_response())?,
            nodes,
        }),
    ))
}

async fn events_handler(
    State(state): State<ServerContext>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.event_sender.subscribe();
    let stream = BroadcastStream::new(receiver).map(|item| {
        let event = match item {
            Ok(event) => event,
            Err(_) => ApiEvent {
                event: "system:lagged".to_string(),
                message: "事件缓冲拥塞，已丢弃部分消息".to_string(),
                source_id: None,
                timestamp: current_timestamp_rfc3339()
                    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
            },
        };
        let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok(Event::default().event(event.event).data(payload))
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

async fn host_validation_middleware(
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

async fn cors_reject_middleware(
    State(_state): State<ServerContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    next.run(request).await
}

async fn rate_limit_middleware(
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

async fn admin_auth_middleware(
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
        .is_some_and(|token| token == state.admin_token.as_str());
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

fn load_plugin_by_route_id(
    repository: &PluginRepository<'_>,
    route_id: &str,
) -> Result<Option<Plugin>, StorageError> {
    if let Some(plugin) = repository.get_by_id(route_id)? {
        return Ok(Some(plugin));
    }
    repository.get_by_plugin_id(route_id)
}

fn source_with_config_to_dto(source: SourceWithConfig) -> SourceDto {
    SourceDto {
        source: source.source,
        config: source.config,
    }
}

fn validate_zip_safety(payload: &[u8]) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| config_error_response("插件包不是合法 zip 文件"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(config_error_response("插件 zip 条目数超过 100"));
    }

    let mut total_uncompressed = 0_u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| config_error_response("读取 zip 条目失败"))?;
        let Some(path) = file.enclosed_name() else {
            return Err(config_error_response("插件包路径非法，包含越界路径"));
        };
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(config_error_response("插件包路径非法，包含 .."));
        }
        total_uncompressed = total_uncompressed.saturating_add(file.size());
        if total_uncompressed > MAX_ZIP_TOTAL_UNCOMPRESSED_BYTES {
            return Err(config_error_response("插件包解压总大小超过 50MB"));
        }
    }
    Ok(())
}

fn extract_zip_to_dir(
    payload: &[u8],
    target_dir: &Path,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| config_error_response("插件包不是合法 zip 文件"))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| config_error_response("读取 zip 条目失败"))?;
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| config_error_response("插件包路径非法，包含越界路径"))?;
        let out_path = target_dir.join(enclosed);

        if file.is_dir() {
            fs::create_dir_all(&out_path).map_err(|_| internal_error_response())?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|_| internal_error_response())?;
        }
        let mut output = fs::File::create(&out_path).map_err(|_| internal_error_response())?;
        std::io::copy(&mut file, &mut output).map_err(|_| internal_error_response())?;
        output.flush().map_err(|_| internal_error_response())?;
    }
    Ok(())
}

fn validate_source_ids_exist(
    db: &Database,
    source_ids: &[String],
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let source_repository = SourceRepository::new(db);
    for source_id in source_ids {
        if source_repository
            .get_by_id(source_id)
            .map_err(storage_error_to_response)?
            .is_none()
        {
            return Err(config_error_response(&format!(
                "source_id 不存在：{source_id}"
            )));
        }
    }
    Ok(())
}

fn replace_profile_sources(
    db: &Database,
    profile_id: &str,
    source_ids: &[String],
) -> Result<(), StorageError> {
    db.with_connection(|connection| {
        let tx = connection.transaction()?;
        tx.execute(
            "DELETE FROM profile_sources WHERE profile_id = ?1",
            [profile_id],
        )?;
        for (index, source_id) in source_ids.iter().enumerate() {
            tx.execute(
                "INSERT INTO profile_sources (profile_id, source_instance_id, priority)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![profile_id, source_id, index as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

fn list_profile_source_ids(db: &Database, profile_id: &str) -> Result<Vec<String>, StorageError> {
    db.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT source_instance_id
             FROM profile_sources
             WHERE profile_id = ?1
             ORDER BY priority, source_instance_id",
        )?;
        let rows = statement.query_map([profile_id], |row| row.get::<_, String>(0))?;
        let mut source_ids = Vec::new();
        for row in rows {
            source_ids.push(row?);
        }
        Ok(source_ids)
    })
}

fn is_valid_export_token(
    db: &Database,
    profile_id: &str,
    token: &str,
) -> Result<bool, StorageError> {
    let now = current_timestamp_rfc3339().map_err(StorageError::Io)?;
    let repository = ExportTokenRepository::new(db);
    repository.is_valid_token(profile_id, token, &now)
}

fn emit_event(state: &ServerContext, event: &str, message: String, source_id: Option<String>) {
    let payload = ApiEvent {
        event: event.to_string(),
        message,
        source_id,
        timestamp: current_timestamp_rfc3339()
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
    };
    let _ = state.event_sender.send(payload);
}

fn current_timestamp_rfc3339() -> Result<String, std::io::Error> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| std::io::Error::other(format!("时间格式化失败：{error}")))
}

fn parse_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    let token = trimmed.strip_prefix("Bearer ")?;
    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}

fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let name = parts.next().unwrap_or_default();
        if name == key {
            let value = parts.next().unwrap_or_default();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn extract_profile_id_from_path(path: &str) -> Option<&str> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() >= 4 && parts[1] == "api" && parts[2] == "profiles" && !parts[3].is_empty() {
        Some(parts[3])
    } else {
        None
    }
}

fn is_profile_read_endpoint(method: &Method, path: &str) -> bool {
    if *method != Method::GET {
        return false;
    }
    let Some(profile_id) = extract_profile_id_from_path(path) else {
        return false;
    };
    let suffix = path.strip_prefix(&format!("/api/profiles/{profile_id}/"));
    matches!(suffix, Some("raw" | "clash" | "sing-box" | "base64"))
}

fn normalize_host(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn map_settings(settings: Vec<AppSetting>) -> BTreeMap<String, String> {
    settings
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect()
}

fn core_error_to_response(error: CoreError) -> (StatusCode, Json<ErrorResponse>) {
    let code = error.code().to_string();
    match error {
        CoreError::ConfigInvalid(message) => {
            error_response(StatusCode::BAD_REQUEST, &code, message, false)
        }
        CoreError::PluginAlreadyInstalled(message) => {
            error_response(StatusCode::CONFLICT, &code, message, false)
        }
        CoreError::PluginNotFound(message) | CoreError::SourceNotFound(message) => {
            error_response(StatusCode::NOT_FOUND, &code, message, false)
        }
        CoreError::SubscriptionFetch(message) => {
            error_response(StatusCode::BAD_GATEWAY, &code, message, true)
        }
        CoreError::SubscriptionParse(message) => {
            error_response(StatusCode::BAD_REQUEST, &code, message, false)
        }
        other => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &code,
            other.to_string(),
            true,
        ),
    }
}

fn storage_error_to_response(error: StorageError) -> (StatusCode, Json<ErrorResponse>) {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "E_INTERNAL",
        format!("存储层错误：{error}"),
        true,
    )
}

fn unauthorized_error_response() -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::UNAUTHORIZED, "E_AUTH", "Unauthorized", false)
}

fn not_found_error_response(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::NOT_FOUND, "E_NOT_FOUND", message, false)
}

fn config_error_response(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::BAD_REQUEST, "E_CONFIG_INVALID", message, false)
}

fn internal_error_response() -> (StatusCode, Json<ErrorResponse>) {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "E_INTERNAL",
        "Internal server error",
        true,
    )
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
    retryable: bool,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse::new(code, message.into(), retryable)),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use app_secrets::MemorySecretStore;
    use app_storage::{ExportTokenRepository, RefreshJobRepository};
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header::CONTENT_TYPE, header::HOST};
    use axum::routing::get;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use tower::ServiceExt;
    use zip::write::SimpleFileOptions;

    use super::{ApiEvent, ServerContext, build_router};

    fn build_test_state() -> ServerContext {
        let database = Arc::new(app_storage::Database::open_in_memory().expect("初始化数据库失败"));
        let secret_store = Arc::new(MemorySecretStore::new());
        let plugins_dir = std::env::temp_dir().join(format!(
            "subforge-http-server-test-{}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        std::fs::create_dir_all(&plugins_dir).expect("创建测试插件目录失败");
        let (tx, _rx) = tokio::sync::broadcast::channel::<ApiEvent>(64);
        ServerContext::new(
            "test-admin-token".to_string(),
            database,
            secret_store,
            plugins_dir,
            18118,
            tx,
        )
    }

    #[tokio::test]
    async fn plugins_api_requires_admin_token() {
        let app = build_router(build_test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/plugins")
                    .header(HOST, "127.0.0.1:18118")
                    .body(Body::empty())
                    .expect("创建请求失败"),
            )
            .await
            .expect("请求执行失败");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plugins_api_rejects_query_admin_token() {
        let app = build_router(build_test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/plugins?token=test-admin-token")
                    .header(HOST, "127.0.0.1:18118")
                    .body(Body::empty())
                    .expect("创建请求失败"),
            )
            .await
            .expect("请求执行失败");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plugins_api_accepts_admin_header() {
        let app = build_router(build_test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/plugins")
                    .header(HOST, "127.0.0.1:18118")
                    .header("authorization", "Bearer test-admin-token")
                    .body(Body::empty())
                    .expect("创建请求失败"),
            )
            .await
            .expect("请求执行失败");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 64)
            .await
            .expect("读取响应体失败");
        let raw = String::from_utf8(body.to_vec()).expect("响应体不是 UTF-8");
        assert!(raw.contains("\"plugins\""));
    }

    #[tokio::test]
    async fn options_preflight_returns_204_without_cors_header() {
        let app = build_router(build_test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/plugins")
                    .header(HOST, "127.0.0.1:18118")
                    .body(Body::empty())
                    .expect("创建请求失败"),
            )
            .await
            .expect("请求执行失败");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
    }

    #[tokio::test]
    async fn e2e_import_source_refresh_and_raw_profile_output() {
        let state = build_test_state();
        let mut event_receiver = state.event_sender.subscribe();
        let app = build_router(state.clone());

        let (upstream_base, server_task) = start_fixture_server(
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;

        let boundary = "----subforge-e2e-boundary";
        let plugin_zip = build_builtin_plugin_zip_bytes();
        let import_body = build_multipart_plugin_body(boundary, &plugin_zip, "builtin-static.zip");
        let import_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/plugins/import")
                    .header(HOST, "127.0.0.1:18118")
                    .header("authorization", "Bearer test-admin-token")
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(import_body))
                    .expect("构建导入插件请求失败"),
            )
            .await
            .expect("导入插件请求执行失败");
        assert_eq!(import_response.status(), StatusCode::CREATED);

        let source_response = app
            .clone()
            .oneshot(admin_json_request(
                Method::POST,
                "/api/sources",
                &json!({
                    "plugin_id": "subforge.builtin.static",
                    "name": "E2E Source",
                    "config": {
                        "url": format!("{upstream_base}/sub")
                    }
                }),
            ))
            .await
            .expect("创建来源请求执行失败");
        assert_eq!(source_response.status(), StatusCode::CREATED);
        let source_payload = read_json(source_response).await;
        let source_id = source_payload
            .pointer("/source/source/id")
            .and_then(Value::as_str)
            .expect("来源响应缺少 source.id")
            .to_string();

        let profile_response = app
            .clone()
            .oneshot(admin_json_request(
                Method::POST,
                "/api/profiles",
                &json!({
                    "name": "E2E Profile",
                    "source_ids": [source_id.clone()]
                }),
            ))
            .await
            .expect("创建 Profile 请求执行失败");
        assert_eq!(profile_response.status(), StatusCode::CREATED);
        let profile_payload = read_json(profile_response).await;
        let profile_id = profile_payload
            .pointer("/profile/profile/id")
            .and_then(Value::as_str)
            .expect("Profile 响应缺少 id")
            .to_string();

        let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
        let export_token = export_token_repository
            .get_active_token(&profile_id)
            .expect("读取 export_token 失败")
            .expect("创建 Profile 后应自动生成 export_token")
            .token;

        let refresh_response = app
            .clone()
            .oneshot(admin_request(
                Method::POST,
                &format!("/api/sources/{source_id}/refresh"),
                Body::empty(),
            ))
            .await
            .expect("刷新来源请求执行失败");
        assert_eq!(refresh_response.status(), StatusCode::OK);
        let refresh_payload = read_json(refresh_response).await;
        assert_eq!(
            refresh_payload.get("source_id").and_then(Value::as_str),
            Some(source_id.as_str())
        );
        assert_eq!(
            refresh_payload.get("node_count").and_then(Value::as_u64),
            Some(3)
        );

        let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
        let refresh_jobs = refresh_repository
            .list_by_source(&source_id)
            .expect("读取 refresh_jobs 失败");
        assert_eq!(refresh_jobs.len(), 1);
        assert_eq!(refresh_jobs[0].status, "success");
        assert_eq!(refresh_jobs[0].node_count, Some(3));

        let event = wait_refresh_complete_event(&mut event_receiver, &source_id).await;
        assert_eq!(event.event, "refresh:complete");
        assert_eq!(event.source_id.as_deref(), Some(source_id.as_str()));

        let raw_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/profiles/{profile_id}/raw?token={export_token}"
                    ))
                    .header(HOST, "127.0.0.1:18118")
                    .body(Body::empty())
                    .expect("构建 raw 请求失败"),
            )
            .await
            .expect("读取 raw 订阅请求执行失败");
        assert_eq!(raw_response.status(), StatusCode::OK);
        let raw_payload = read_json(raw_response).await;
        assert_eq!(
            raw_payload.get("profile_id").and_then(Value::as_str),
            Some(profile_id.as_str())
        );
        assert_eq!(
            raw_payload.get("node_count").and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            raw_payload
                .get("nodes")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(3)
        );

        server_task.abort();
    }

    #[tokio::test]
    async fn e2e_script_source_refresh_via_management_api() {
        let state = build_test_state();
        let mut event_receiver = state.event_sender.subscribe();
        let app = build_router(state.clone());

        let (upstream_base, server_task) = start_fixture_server(
            BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
            "text/plain; charset=utf-8",
        )
        .await;

        let boundary = "----subforge-e2e-script-boundary";
        let plugin_zip = build_script_mock_plugin_zip_bytes();
        let import_body = build_multipart_plugin_body(boundary, &plugin_zip, "script-mock.zip");
        let import_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/plugins/import")
                    .header(HOST, "127.0.0.1:18118")
                    .header("authorization", "Bearer test-admin-token")
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(import_body))
                    .expect("构建脚本插件导入请求失败"),
            )
            .await
            .expect("导入脚本插件请求执行失败");
        assert_eq!(import_response.status(), StatusCode::CREATED);

        let source_response = app
            .clone()
            .oneshot(admin_json_request(
                Method::POST,
                "/api/sources",
                &json!({
                    "plugin_id": "vendor.example.script-mock",
                    "name": "Script E2E Source",
                    "config": {
                        "subscription_url": format!("{upstream_base}/sub"),
                        "username": "alice",
                        "password": "wonderland"
                    }
                }),
            ))
            .await
            .expect("创建脚本来源请求执行失败");
        assert_eq!(source_response.status(), StatusCode::CREATED);
        let source_payload = read_json(source_response).await;
        let source_id = source_payload
            .pointer("/source/source/id")
            .and_then(Value::as_str)
            .expect("脚本来源响应缺少 source.id")
            .to_string();

        let profile_response = app
            .clone()
            .oneshot(admin_json_request(
                Method::POST,
                "/api/profiles",
                &json!({
                    "name": "Script E2E Profile",
                    "source_ids": [source_id.clone()]
                }),
            ))
            .await
            .expect("创建脚本 Profile 请求执行失败");
        assert_eq!(profile_response.status(), StatusCode::CREATED);
        let profile_payload = read_json(profile_response).await;
        let profile_id = profile_payload
            .pointer("/profile/profile/id")
            .and_then(Value::as_str)
            .expect("脚本 Profile 响应缺少 id")
            .to_string();

        let export_token_repository = ExportTokenRepository::new(state.database.as_ref());
        let export_token = export_token_repository
            .get_active_token(&profile_id)
            .expect("读取脚本 Profile export_token 失败")
            .expect("创建脚本 Profile 后应自动生成 export_token")
            .token;

        let refresh_response = app
            .clone()
            .oneshot(admin_request(
                Method::POST,
                &format!("/api/sources/{source_id}/refresh"),
                Body::empty(),
            ))
            .await
            .expect("刷新脚本来源请求执行失败");
        let refresh_status = refresh_response.status();
        let refresh_payload = read_json(refresh_response).await;
        assert_eq!(
            refresh_status,
            StatusCode::OK,
            "脚本来源刷新应成功，实际返回：{refresh_payload:?}"
        );
        assert_eq!(
            refresh_payload.get("source_id").and_then(Value::as_str),
            Some(source_id.as_str())
        );
        assert_eq!(
            refresh_payload.get("node_count").and_then(Value::as_u64),
            Some(3)
        );

        let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
        let refresh_jobs = refresh_repository
            .list_by_source(&source_id)
            .expect("读取脚本 refresh_jobs 失败");
        assert_eq!(refresh_jobs.len(), 1);
        assert_eq!(refresh_jobs[0].status, "success");
        assert_eq!(refresh_jobs[0].node_count, Some(3));

        let event = wait_refresh_complete_event(&mut event_receiver, &source_id).await;
        assert_eq!(event.event, "refresh:complete");
        assert_eq!(event.source_id.as_deref(), Some(source_id.as_str()));

        let raw_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/profiles/{profile_id}/raw?token={export_token}"
                    ))
                    .header(HOST, "127.0.0.1:18118")
                    .body(Body::empty())
                    .expect("构建脚本 raw 请求失败"),
            )
            .await
            .expect("读取脚本 raw 订阅请求执行失败");
        assert_eq!(raw_response.status(), StatusCode::OK);
        let raw_payload = read_json(raw_response).await;
        assert_eq!(
            raw_payload.get("profile_id").and_then(Value::as_str),
            Some(profile_id.as_str())
        );
        assert_eq!(
            raw_payload.get("node_count").and_then(Value::as_u64),
            Some(3)
        );

        let source_repository = app_storage::SourceRepository::new(state.database.as_ref());
        let persisted_state_raw = source_repository
            .get_by_id(&source_id)
            .expect("读取脚本来源失败")
            .and_then(|source| source.state_json)
            .expect("脚本来源刷新后应写入 state_json");
        let persisted_state: Value =
            serde_json::from_str(&persisted_state_raw).expect("state_json 必须是合法 JSON");
        assert_eq!(
            persisted_state.get("counter").and_then(Value::as_u64),
            Some(3)
        );

        server_task.abort();
    }

    fn admin_request(method: Method, uri: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(HOST, "127.0.0.1:18118")
            .header("authorization", "Bearer test-admin-token")
            .body(body)
            .expect("构建管理请求失败")
    }

    fn admin_json_request(method: Method, uri: &str, payload: &Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(HOST, "127.0.0.1:18118")
            .header("authorization", "Bearer test-admin-token")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(payload).expect("序列化 JSON 请求体失败"),
            ))
            .expect("构建管理 JSON 请求失败")
    }

    async fn read_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("读取响应体失败");
        serde_json::from_slice::<Value>(&body).expect("响应体不是合法 JSON")
    }

    async fn wait_refresh_complete_event(
        receiver: &mut tokio::sync::broadcast::Receiver<ApiEvent>,
        source_id: &str,
    ) -> ApiEvent {
        timeout(Duration::from_secs(5), async {
            loop {
                let event = receiver.recv().await.expect("读取事件失败");
                if event.event == "refresh:complete"
                    && event.source_id.as_deref() == Some(source_id)
                {
                    return event;
                }
            }
        })
        .await
        .expect("等待 refresh:complete 事件超时")
    }

    fn build_builtin_plugin_zip_bytes() -> Vec<u8> {
        let plugin_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for file_name in ["plugin.json", "schema.json"] {
                writer
                    .start_file(file_name, options)
                    .expect("写入 zip 条目失败");
                let bytes = fs::read(plugin_dir.join(file_name)).expect("读取内置插件文件失败");
                writer.write_all(&bytes).expect("写入 zip 数据失败");
            }
            writer.finish().expect("完成 zip 构建失败");
        }
        cursor.into_inner()
    }

    fn script_mock_plugin_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/script-mock")
    }

    fn build_script_mock_plugin_zip_bytes() -> Vec<u8> {
        let plugin_dir = script_mock_plugin_dir();
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for file_name in [
                "plugin.json",
                "schema.json",
                "scripts/login.lua",
                "scripts/refresh.lua",
                "scripts/fetch.lua",
            ] {
                writer
                    .start_file(file_name, options)
                    .expect("写入脚本插件 zip 条目失败");
                let bytes = fs::read(plugin_dir.join(file_name)).expect("读取脚本插件文件失败");
                writer.write_all(&bytes).expect("写入脚本插件 zip 数据失败");
            }
            writer.finish().expect("完成脚本插件 zip 构建失败");
        }
        cursor.into_inner()
    }

    fn build_multipart_plugin_body(boundary: &str, zip_payload: &[u8], filename: &str) -> Vec<u8> {
        let mut body = Vec::new();
        write!(body, "--{boundary}\r\n").expect("写入 multipart 边界失败");
        write!(
            body,
            "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
        )
        .expect("写入 multipart disposition 失败");
        write!(body, "Content-Type: application/zip\r\n\r\n")
            .expect("写入 multipart content-type 失败");
        body.extend_from_slice(zip_payload);
        write!(body, "\r\n--{boundary}--\r\n").expect("写入 multipart 结束边界失败");
        body
    }

    async fn start_fixture_server(
        body: String,
        content_type: &'static str,
    ) -> (String, JoinHandle<()>) {
        let app = Router::new().route(
            "/sub",
            get(move || {
                let body = body.clone();
                async move { ([(axum::http::header::CONTENT_TYPE, content_type)], body) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("启动测试上游服务器失败");
        let address: SocketAddr = listener.local_addr().expect("读取测试监听地址失败");
        let base_url = format!("http://{address}");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试上游服务器运行失败");
        });
        (base_url, handle)
    }

    const BASE64_SUBSCRIPTION_FIXTURE: &str =
        include_str!("../../app-core/tests/fixtures/subscription_base64.txt");
}
