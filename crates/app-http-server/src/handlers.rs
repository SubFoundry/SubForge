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
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::ApiEvent;
use crate::helpers::{
    config_error_response, core_error_to_response, current_timestamp_rfc3339, emit_event,
    error_response, extract_zip_to_dir, internal_error_response, list_profile_source_ids,
    load_plugin_by_route_id, map_settings, not_found_error_response, replace_profile_sources,
    source_with_config_to_dto, storage_error_to_response, validate_source_ids_exist,
    validate_zip_safety,
};
use crate::state::{
    APP_VERSION, ApiResult, HealthResponse, MAX_PLUGIN_UPLOAD_BYTES, ServerContext,
};
#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSettingsRequest {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PluginListResponse {
    plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SourceDto {
    pub(crate) source: SourceInstance,
    pub(crate) config: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceListResponse {
    sources: Vec<SourceDto>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceResponse {
    source: SourceDto,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSourceRequest {
    plugin_id: String,
    name: String,
    #[serde(default)]
    config: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSourceRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    config: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshSourceResponse {
    source_id: String,
    node_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProfileDto {
    profile: Profile,
    source_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileListResponse {
    profiles: Vec<ProfileDto>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileResponse {
    profile: ProfileDto,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateProfileRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateProfileRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<Option<String>>,
    #[serde(default)]
    source_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenQuery {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileRawResponse {
    profile_id: String,
    profile_name: String,
    node_count: usize,
    generated_at: String,
    nodes: Vec<ProxyNode>,
}

pub(crate) async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            version: APP_VERSION,
        }),
    )
}

pub(crate) async fn get_system_settings_handler(
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

pub(crate) async fn update_system_settings_handler(
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

pub(crate) async fn list_plugins_handler(
    State(state): State<ServerContext>,
) -> ApiResult<PluginListResponse> {
    let repository = PluginRepository::new(state.database.as_ref());
    let plugins = repository.list().map_err(storage_error_to_response)?;
    Ok((StatusCode::OK, Json(PluginListResponse { plugins })))
}

pub(crate) async fn import_plugin_handler(
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

pub(crate) async fn delete_plugin_handler(
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

pub(crate) async fn list_sources_handler(
    State(state): State<ServerContext>,
) -> ApiResult<SourceListResponse> {
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

pub(crate) async fn create_source_handler(
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

pub(crate) async fn update_source_handler(
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

pub(crate) async fn delete_source_handler(
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

pub(crate) async fn refresh_source_handler(
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

pub(crate) async fn list_profiles_handler(
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

pub(crate) async fn create_profile_handler(
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

pub(crate) async fn update_profile_handler(
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

pub(crate) async fn delete_profile_handler(
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

pub(crate) async fn get_profile_raw_handler(
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

pub(crate) async fn events_handler(
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
