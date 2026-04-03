use app_transform::{Base64Transformer, ClashTransformer, SingboxTransformer, Transformer};

use super::*;

pub(crate) async fn get_profile_clash_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenQuery>,
) -> ApiResponseResult {
    let _ = query.token.as_deref();
    let cache_entry = load_profile_cache_entry(&state, &id)?;
    let body = ClashTransformer::default()
        .transform(&cache_entry.nodes, &cache_entry.profile)
        .map_err(transform_error_to_response)?;
    let headers = build_subscription_headers(
        state.database.as_ref(),
        &cache_entry,
        "yaml",
        "text/yaml; charset=utf-8",
    )?;
    Ok((StatusCode::OK, headers, body).into_response())
}

pub(crate) async fn get_profile_singbox_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenQuery>,
) -> ApiResponseResult {
    let _ = query.token.as_deref();
    let cache_entry = load_profile_cache_entry(&state, &id)?;
    let body = SingboxTransformer::default()
        .transform(&cache_entry.nodes, &cache_entry.profile)
        .map_err(transform_error_to_response)?;
    let headers = build_subscription_headers(
        state.database.as_ref(),
        &cache_entry,
        "json",
        "application/json; charset=utf-8",
    )?;
    Ok((StatusCode::OK, headers, body).into_response())
}

pub(crate) async fn get_profile_base64_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenQuery>,
) -> ApiResponseResult {
    let _ = query.token.as_deref();
    let cache_entry = load_profile_cache_entry(&state, &id)?;
    let body = Base64Transformer
        .transform(&cache_entry.nodes, &cache_entry.profile)
        .map_err(transform_error_to_response)?;
    let headers = build_subscription_headers(
        state.database.as_ref(),
        &cache_entry,
        "txt",
        "text/plain; charset=utf-8",
    )?;
    Ok((StatusCode::OK, headers, body).into_response())
}

pub(crate) async fn get_profile_raw_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenQuery>,
) -> ApiResponseResult {
    let _ = query.token.as_deref();
    let cache_entry = load_profile_cache_entry(&state, &id)?;

    let payload = ProfileRawResponse {
        profile_id: cache_entry.profile.id.clone(),
        profile_name: cache_entry.profile.name.clone(),
        node_count: cache_entry.nodes.len(),
        generated_at: cache_entry.generated_at.clone(),
        nodes: cache_entry.nodes.clone(),
    };
    let body = serde_json::to_string(&payload).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "E_INTERNAL",
            "序列化失败",
            true,
        )
    })?;
    let headers = build_subscription_headers(
        state.database.as_ref(),
        &cache_entry,
        "json",
        "application/json; charset=utf-8",
    )?;
    Ok((StatusCode::OK, headers, body).into_response())
}
