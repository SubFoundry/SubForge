use super::*;

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

pub(crate) async fn get_system_status_handler(
    State(state): State<ServerContext>,
) -> ApiResult<SystemStatusResponse> {
    let source_repository = SourceRepository::new(state.database.as_ref());
    let cache_repository = NodeCacheRepository::new(state.database.as_ref());
    let refresh_repository = RefreshJobRepository::new(state.database.as_ref());

    let sources = source_repository
        .list()
        .map_err(storage_error_to_response)?;
    let active_sources = sources
        .iter()
        .filter(|source| source.status != "disabled")
        .count();

    let mut total_nodes = 0usize;
    let mut last_refresh_at: Option<String> = None;
    for source in &sources {
        if let Some(entry) = cache_repository
            .get_by_source(&source.id)
            .map_err(storage_error_to_response)?
        {
            total_nodes = total_nodes.saturating_add(entry.nodes.len());
        }

        let jobs = refresh_repository
            .list_by_source(&source.id)
            .map_err(storage_error_to_response)?;
        for job in jobs {
            if let Some(finished_at) = job.finished_at {
                let should_update = match last_refresh_at.as_deref() {
                    Some(current) => finished_at.as_str() > current,
                    None => true,
                };
                if should_update {
                    last_refresh_at = Some(finished_at);
                }
            }
        }
    }

    Ok((
        StatusCode::OK,
        Json(SystemStatusResponse {
            active_sources,
            total_nodes,
            last_refresh_at,
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
