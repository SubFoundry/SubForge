use std::time::Duration;

use app_aggregator::{Aggregator, SourceNodes};
use app_common::ErrorResponse;
use app_transform::TransformError;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::state::{PROFILE_CACHE_TTL_SECONDS, ProfileCacheEntry};

use super::*;

mod export;
mod read;
mod tokens;
mod write;

pub(crate) use export::{
    get_profile_base64_handler, get_profile_clash_handler, get_profile_raw_handler,
    get_profile_singbox_handler,
};
pub(crate) use read::list_profiles_handler;
pub(crate) use tokens::rotate_profile_export_token_handler;
pub(crate) use write::{
    create_profile_handler, delete_profile_handler, refresh_profile_handler, update_profile_handler,
};

const PROFILE_TITLE_HEADER: &str = "profile-title";
const PROFILE_UPDATE_INTERVAL_HEADER: &str = "profile-update-interval";
const SUBSCRIPTION_USERINFO_HEADER: &str = "subscription-userinfo";
const DEFAULT_PROFILE_UPDATE_INTERVAL_HOURS: u64 = 24;

fn load_profile_cache_entry(
    state: &ServerContext,
    profile_id: &str,
) -> Result<ProfileCacheEntry, (StatusCode, Json<ErrorResponse>)> {
    let ttl = Duration::from_secs(PROFILE_CACHE_TTL_SECONDS);
    if let Some(entry) = state.profile_cache.get_fresh(profile_id, ttl) {
        return Ok(entry);
    }

    let profile_repository = ProfileRepository::new(state.database.as_ref());
    let source_repository = SourceRepository::new(state.database.as_ref());
    let cache_repository = NodeCacheRepository::new(state.database.as_ref());

    let profile = profile_repository
        .get_by_id(profile_id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("Profile 不存在"))?;
    let source_ids = list_profile_source_ids(state.database.as_ref(), profile_id)
        .map_err(storage_error_to_response)?;

    let mut source_nodes = Vec::with_capacity(source_ids.len());
    for source_id in &source_ids {
        let alias = source_repository
            .get_by_id(source_id)
            .map_err(storage_error_to_response)?
            .map(|source| source.name);
        let nodes = cache_repository
            .get_by_source(source_id)
            .map_err(storage_error_to_response)?
            .map(|entry| entry.nodes)
            .unwrap_or_default();
        let source_bucket = if let Some(alias) = alias {
            SourceNodes::with_alias(source_id.clone(), alias, nodes)
        } else {
            SourceNodes::new(source_id.clone(), nodes)
        };
        source_nodes.push(source_bucket);
    }

    let aggregation = Aggregator.aggregate(&source_nodes);
    let subscription_userinfo = if source_ids.len() == 1 {
        state.source_userinfo_cache.get(&source_ids[0])
    } else {
        None
    };
    let generated_at = current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
    let entry = ProfileCacheEntry::with_cached_at(
        profile.clone(),
        source_ids,
        aggregation.nodes,
        generated_at,
        subscription_userinfo,
    );

    state.profile_cache.insert(&profile.id, entry.clone());
    Ok(entry)
}

fn build_subscription_headers(
    database: &app_storage::Database,
    cache_entry: &ProfileCacheEntry,
    extension: &str,
    content_type: &'static str,
) -> Result<HeaderMap, (StatusCode, Json<ErrorResponse>)> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));

    let filename = build_attachment_filename(
        &cache_entry.profile.name,
        &cache_entry.profile.id,
        extension,
    );
    let disposition = format!("attachment; filename=\"{filename}\"");
    let disposition = HeaderValue::from_str(&disposition).map_err(|_| internal_error_response())?;
    headers.insert(CONTENT_DISPOSITION, disposition);

    let profile_title_name = HeaderName::from_static(PROFILE_TITLE_HEADER);
    let profile_title = encode_header_value(&cache_entry.profile.name);
    let profile_title =
        HeaderValue::from_str(&profile_title).map_err(|_| internal_error_response())?;
    headers.insert(profile_title_name, profile_title);

    let interval_hours =
        resolve_profile_update_interval_hours(database, cache_entry.profile.id.as_str());
    let interval_name = HeaderName::from_static(PROFILE_UPDATE_INTERVAL_HEADER);
    let interval_value = HeaderValue::from_str(&interval_hours.to_string())
        .map_err(|_| internal_error_response())?;
    headers.insert(interval_name, interval_value);

    if cache_entry.source_ids.len() == 1
        && let Some(userinfo) = cache_entry.subscription_userinfo.as_deref()
    {
        let userinfo_name = HeaderName::from_static(SUBSCRIPTION_USERINFO_HEADER);
        let encoded = encode_header_value(userinfo);
        let userinfo_value =
            HeaderValue::from_str(&encoded).map_err(|_| internal_error_response())?;
        headers.insert(userinfo_name, userinfo_value);
    }

    Ok(headers)
}

fn resolve_profile_update_interval_hours(
    database: &app_storage::Database,
    profile_id: &str,
) -> u64 {
    let repository = SettingsRepository::new(database);
    let profile_key = format!("profile.{profile_id}.update_interval_hours");

    for key in [
        profile_key.as_str(),
        "profile.default_update_interval_hours",
    ] {
        let Ok(Some(setting)) = repository.get(key) else {
            continue;
        };
        let Ok(value) = setting.value.trim().parse::<u64>() else {
            continue;
        };
        if value > 0 {
            return value;
        }
    }

    DEFAULT_PROFILE_UPDATE_INTERVAL_HOURS
}

fn transform_error_to_response(error: TransformError) -> (StatusCode, Json<ErrorResponse>) {
    error_response(
        StatusCode::BAD_REQUEST,
        error.code(),
        error.to_string(),
        false,
    )
}

fn build_attachment_filename(profile_name: &str, profile_id: &str, extension: &str) -> String {
    let mut stem = sanitize_filename_component(profile_name);
    if stem.is_empty() {
        let fallback = sanitize_filename_component(profile_id);
        stem = if fallback.is_empty() {
            "profile".to_string()
        } else {
            fallback
        };
    }
    format!("{stem}.{extension}")
}

fn sanitize_filename_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn encode_header_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "subforge".to_string();
    }
    if HeaderValue::from_str(trimmed).is_ok() {
        return trimmed.to_string();
    }

    let mut encoded = String::with_capacity(trimmed.len() * 3);
    for byte in trimmed.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b' ') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(nibble_to_hex(byte >> 4));
            encoded.push(nibble_to_hex(byte & 0x0F));
        }
    }
    encoded
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + (value - 10)),
    }
}

fn build_profile_dto(
    database: &app_storage::Database,
    profile: Profile,
    source_ids: Vec<String>,
) -> Result<ProfileDto, (StatusCode, Json<ErrorResponse>)> {
    let export_token_repository = ExportTokenRepository::new(database);
    let export_token = export_token_repository
        .get_active_token(&profile.id)
        .map_err(storage_error_to_response)?
        .map(|token| token.token);

    Ok(ProfileDto {
        profile,
        source_ids,
        export_token,
    })
}
