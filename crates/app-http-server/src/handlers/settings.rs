use super::*;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::fs;
use std::path::Path;

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

pub(crate) async fn shutdown_system_handler(
    State(state): State<ServerContext>,
) -> ApiResult<ShutdownResponse> {
    let _ = state.shutdown_signal.send(true);
    Ok((StatusCode::OK, Json(ShutdownResponse { accepted: true })))
}

pub(crate) async fn rotate_admin_token_handler(
    State(state): State<ServerContext>,
) -> ApiResult<RotateAdminTokenResponse> {
    let token = generate_admin_token().map_err(|_| internal_error_response())?;
    persist_admin_token(state.admin_token_path.as_path(), &token)
        .map_err(|_| internal_error_response())?;
    {
        let mut guard = state
            .admin_token
            .write()
            .map_err(|_| internal_error_response())?;
        *guard = token.clone();
    }
    state.auth_failures.reset();
    emit_event(
        &state,
        "system:admin-token-rotated",
        "admin token 已轮换".to_string(),
        None,
    );
    Ok((StatusCode::OK, Json(RotateAdminTokenResponse { token })))
}

fn generate_admin_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn persist_admin_token(path: &Path, token: &str) -> std::io::Result<()> {
    fs::write(path, format!("{token}\n"))?;
    set_owner_only_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(windows)]
fn set_owner_only_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::process::Command;

    let username = std::env::var("USERNAME")
        .map_err(|err| std::io::Error::other(format!("读取 USERNAME 失败: {err}")))?;
    let target = path.to_string_lossy().into_owned();
    let grant = format!("{username}:(R,W)");

    let inheritance = Command::new("icacls")
        .arg(&target)
        .args(["/inheritance:r"])
        .output()?;
    if !inheritance.status.success() {
        return Err(std::io::Error::other(format!(
            "icacls 关闭继承失败: {}",
            String::from_utf8_lossy(&inheritance.stderr)
        )));
    }

    let grant_output = Command::new("icacls")
        .arg(&target)
        .args(["/grant:r", &grant])
        .output()?;
    if !grant_output.status.success() {
        return Err(std::io::Error::other(format!(
            "icacls 授权失败: {}",
            String::from_utf8_lossy(&grant_output.stderr)
        )));
    }

    Ok(())
}
