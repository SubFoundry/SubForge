use super::*;

pub(crate) async fn rotate_profile_export_token_handler(
    State(state): State<ServerContext>,
    AxumPath(profile_id): AxumPath<String>,
) -> ApiResult<RotateProfileExportTokenResponse> {
    let repository = ProfileRepository::new(state.database.as_ref());
    if repository
        .get_by_id(&profile_id)
        .map_err(storage_error_to_response)?
        .is_none()
    {
        return Err(not_found_error_response("Profile 不存在"));
    }

    let engine = Engine::new(
        state.database.as_ref(),
        &state.plugins_dir,
        Arc::clone(&state.secret_store),
    );
    let rotated = engine
        .rotate_profile_export_token(&profile_id)
        .map_err(core_error_to_response)?;

    state.profile_cache.invalidate(&profile_id);
    emit_event(
        &state,
        "profile:token-rotated",
        format!("Profile export token 已轮换：{profile_id}"),
        None,
    );

    Ok((
        StatusCode::OK,
        Json(RotateProfileExportTokenResponse {
            profile_id,
            token: rotated.token,
            previous_token_expires_at: rotated.grace_expires_at,
        }),
    ))
}
