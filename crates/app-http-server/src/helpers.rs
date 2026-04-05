use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Component, Path};

use app_common::{AppSetting, ErrorResponse, Plugin};
use app_core::{CoreError, SourceWithConfig};
use app_plugin_runtime::PluginRuntimeError;
use app_storage::{
    Database, ExportTokenRepository, PluginRepository, SourceRepository, StorageError,
};
use axum::Json;
use axum::http::{Method, StatusCode};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use zip::ZipArchive;

use crate::handlers::SourceDto;
use crate::state::{ApiEvent, MAX_ZIP_ENTRIES, MAX_ZIP_TOTAL_UNCOMPRESSED_BYTES, ServerContext};
pub(crate) fn load_plugin_by_route_id(
    repository: &PluginRepository<'_>,
    route_id: &str,
) -> Result<Option<Plugin>, StorageError> {
    if let Some(plugin) = repository.get_by_id(route_id)? {
        return Ok(Some(plugin));
    }
    repository.get_by_plugin_id(route_id)
}

pub(crate) fn source_with_config_to_dto(source: SourceWithConfig) -> SourceDto {
    SourceDto {
        source: source.source,
        config: source.config,
    }
}

pub(crate) fn validate_zip_safety(payload: &[u8]) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
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
        normalize_zip_entry_path(file.name())?;
        total_uncompressed = total_uncompressed.saturating_add(file.size());
        if total_uncompressed > MAX_ZIP_TOTAL_UNCOMPRESSED_BYTES {
            return Err(config_error_response("插件包解压总大小超过 50MB"));
        }
    }
    Ok(())
}

pub(crate) fn extract_zip_to_dir(
    payload: &[u8],
    target_dir: &Path,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| config_error_response("插件包不是合法 zip 文件"))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| config_error_response("读取 zip 条目失败"))?;
        let entry_path = normalize_zip_entry_path(file.name())?;
        let out_path = target_dir.join(entry_path);

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

fn normalize_zip_entry_path(
    raw_name: &str,
) -> Result<std::path::PathBuf, (StatusCode, Json<ErrorResponse>)> {
    // 兼容 Windows 常见打包工具（如 Compress-Archive）写入的反斜杠分隔符。
    let normalized = raw_name.replace('\\', "/");
    let mut result = std::path::PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => continue,
            Component::ParentDir => return Err(config_error_response("插件包路径非法，包含 ..")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(config_error_response("插件包路径非法，包含越界路径"));
            }
        }
    }

    if result.as_os_str().is_empty() {
        return Err(config_error_response("插件包路径非法，包含越界路径"));
    }

    Ok(result)
}

pub(crate) fn validate_source_ids_exist(
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

pub(crate) fn replace_profile_sources(
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

pub(crate) fn list_profile_source_ids(
    db: &Database,
    profile_id: &str,
) -> Result<Vec<String>, StorageError> {
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

pub(crate) fn list_profile_ids_by_source(
    db: &Database,
    source_id: &str,
) -> Result<Vec<String>, StorageError> {
    db.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT profile_id
             FROM profile_sources
             WHERE source_instance_id = ?1
             ORDER BY profile_id",
        )?;
        let rows = statement.query_map([source_id], |row| row.get::<_, String>(0))?;
        let mut profile_ids = Vec::new();
        for row in rows {
            profile_ids.push(row?);
        }
        Ok(profile_ids)
    })
}

pub(crate) fn is_valid_export_token(
    db: &Database,
    profile_id: &str,
    token: &str,
) -> Result<bool, StorageError> {
    let now = current_timestamp_rfc3339().map_err(StorageError::Io)?;
    let repository = ExportTokenRepository::new(db);
    repository.is_valid_token(profile_id, token, &now)
}

pub(crate) fn emit_event(
    state: &ServerContext,
    event: &str,
    message: String,
    source_id: Option<String>,
) {
    let payload = ApiEvent {
        event: event.to_string(),
        message,
        source_id,
        timestamp: current_timestamp_rfc3339()
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
    };
    let _ = state.event_sender.send(payload);
}

pub(crate) fn current_timestamp_rfc3339() -> Result<String, std::io::Error> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| std::io::Error::other(format!("时间格式化失败：{error}")))
}

pub(crate) fn parse_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    let token = trimmed.strip_prefix("Bearer ")?;
    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}

pub(crate) fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
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

pub(crate) fn extract_profile_id_from_path(path: &str) -> Option<&str> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() >= 4 && parts[1] == "api" && parts[2] == "profiles" && !parts[3].is_empty() {
        Some(parts[3])
    } else {
        None
    }
}

pub(crate) fn is_profile_read_endpoint(method: &Method, path: &str) -> bool {
    if *method != Method::GET {
        return false;
    }
    let Some(profile_id) = extract_profile_id_from_path(path) else {
        return false;
    };
    let suffix = path.strip_prefix(&format!("/api/profiles/{profile_id}/"));
    matches!(suffix, Some("raw" | "clash" | "sing-box" | "base64"))
}

pub(crate) fn normalize_host(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub(crate) fn map_settings(settings: Vec<AppSetting>) -> BTreeMap<String, String> {
    settings
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect()
}

pub(crate) fn core_error_to_response(error: CoreError) -> (StatusCode, Json<ErrorResponse>) {
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
        CoreError::PluginRuntime(PluginRuntimeError::ScriptTimeout(message))
        | CoreError::PluginRuntime(PluginRuntimeError::ScriptLimit(message))
        | CoreError::PluginRuntime(PluginRuntimeError::ScriptRuntime(message)) => {
            error_response(StatusCode::BAD_REQUEST, &code, message, true)
        }
        CoreError::PluginRuntime(PluginRuntimeError::Incompatible(message))
        | CoreError::PluginRuntime(PluginRuntimeError::Invalid(message)) => {
            error_response(StatusCode::BAD_REQUEST, &code, message, false)
        }
        CoreError::PluginRuntime(PluginRuntimeError::ManifestParse(error)) => error_response(
            StatusCode::BAD_REQUEST,
            &code,
            format!("plugin.json 解析失败：{error}"),
            false,
        ),
        CoreError::PluginRuntime(PluginRuntimeError::SchemaParse(error)) => error_response(
            StatusCode::BAD_REQUEST,
            &code,
            format!("schema.json 解析失败：{error}"),
            false,
        ),
        CoreError::PluginRuntime(PluginRuntimeError::Io(_)) => error_response(
            StatusCode::BAD_REQUEST,
            &code,
            "读取插件文件失败，请确认插件包结构完整（plugin.json/schema.json/脚本文件）",
            false,
        ),
        CoreError::Transport(_) => error_response(
            StatusCode::BAD_GATEWAY,
            &code,
            "Upstream request failed",
            true,
        ),
        CoreError::Storage(_)
        | CoreError::Secret(_)
        | CoreError::Io(_)
        | CoreError::TimeFormat(_)
        | CoreError::Random(_) => internal_error_response(),
    }
}

pub(crate) fn storage_error_to_response(error: StorageError) -> (StatusCode, Json<ErrorResponse>) {
    let _ = error;
    internal_error_response()
}

pub(crate) fn unauthorized_error_response() -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::UNAUTHORIZED, "E_AUTH", "Unauthorized", false)
}

pub(crate) fn not_found_error_response(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::NOT_FOUND, "E_NOT_FOUND", message, false)
}

pub(crate) fn config_error_response(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::BAD_REQUEST, "E_CONFIG_INVALID", message, false)
}

pub(crate) fn internal_error_response() -> (StatusCode, Json<ErrorResponse>) {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "E_INTERNAL",
        "Internal server error",
        true,
    )
}

pub(crate) fn error_response(
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
