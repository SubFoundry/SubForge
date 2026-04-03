use super::*;

pub(crate) async fn list_profiles_handler(
    State(state): State<ServerContext>,
) -> ApiResult<ProfileListResponse> {
    let repository = ProfileRepository::new(state.database.as_ref());
    let profiles = repository.list().map_err(storage_error_to_response)?;
    let mut items = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let source_ids = list_profile_source_ids(state.database.as_ref(), &profile.id)
            .map_err(storage_error_to_response)?;
        items.push(build_profile_dto(
            state.database.as_ref(),
            profile,
            source_ids,
        )?);
    }
    Ok((
        StatusCode::OK,
        Json(ProfileListResponse { profiles: items }),
    ))
}
