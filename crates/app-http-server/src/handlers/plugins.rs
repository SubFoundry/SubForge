use super::*;
use app_common::ErrorResponse;
use app_plugin_runtime::PluginLoader;
use std::path::{Path, PathBuf};

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
    let plugin_root_dir = resolve_plugin_root_dir(temp_dir.path())?;

    let service = PluginInstallService::new(state.database.as_ref(), &state.plugins_dir);
    let installed = service
        .install_from_dir(&plugin_root_dir)
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

pub(crate) async fn toggle_plugin_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<TogglePluginRequest>,
) -> ApiResult<Plugin> {
    let repository = PluginRepository::new(state.database.as_ref());
    let plugin = load_plugin_by_route_id(&repository, &id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("插件不存在"))?;

    let target_status = if payload.enabled {
        "enabled"
    } else {
        "disabled"
    };

    if plugin.status != target_status {
        let updated_at = current_timestamp_rfc3339().map_err(|_| internal_error_response())?;
        repository
            .update_status(&plugin.id, target_status, &updated_at)
            .map_err(storage_error_to_response)?;
    }

    let updated = repository
        .get_by_id(&plugin.id)
        .map_err(storage_error_to_response)?
        .ok_or_else(internal_error_response)?;

    let action = if payload.enabled { "启用" } else { "禁用" };
    emit_event(
        &state,
        "plugin:toggled",
        format!("插件已{action}：{}", updated.plugin_id),
        None,
    );

    Ok((StatusCode::OK, Json(updated)))
}

pub(crate) async fn get_plugin_schema_handler(
    State(state): State<ServerContext>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<PluginSchemaResponse> {
    let repository = PluginRepository::new(state.database.as_ref());
    let plugin = load_plugin_by_route_id(&repository, &id)
        .map_err(storage_error_to_response)?
        .ok_or_else(|| not_found_error_response("插件不存在"))?;

    let plugin_dir = state.plugins_dir.join(&plugin.plugin_id);
    let loaded = PluginLoader::new()
        .load_from_dir(plugin_dir)
        .map_err(|error| {
            error_response(
                StatusCode::BAD_REQUEST,
                "E_PLUGIN_INVALID",
                error.to_string(),
                false,
            )
        })?;

    Ok((
        StatusCode::OK,
        Json(PluginSchemaResponse {
            plugin_id: plugin.plugin_id,
            name: plugin.name,
            plugin_type: plugin.plugin_type,
            secret_fields: loaded.manifest.secret_fields,
            schema: loaded.schema,
        }),
    ))
}

fn resolve_plugin_root_dir(
    extract_root: &Path,
) -> Result<PathBuf, (StatusCode, Json<ErrorResponse>)> {
    if extract_root.join("plugin.json").is_file() {
        return Ok(extract_root.to_path_buf());
    }

    let mut queue = vec![extract_root.to_path_buf()];
    let mut candidates = Vec::new();

    while let Some(dir) = queue.pop() {
        let entries = fs::read_dir(&dir).map_err(|_| internal_error_response())?;
        for entry in entries {
            let entry = entry.map_err(|_| internal_error_response())?;
            let entry_path = entry.path();
            let file_type = entry.file_type().map_err(|_| internal_error_response())?;

            if file_type.is_dir() {
                queue.push(entry_path);
                continue;
            }

            if file_type.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("plugin.json"))
            {
                candidates.push(dir.clone());
            }
        }
    }

    candidates.sort_unstable();
    candidates.dedup();

    match candidates.len() {
        0 => {
            let mut queue = vec![extract_root.to_path_buf()];
            let mut files = Vec::new();
            while let Some(dir) = queue.pop() {
                let entries = fs::read_dir(&dir).map_err(|_| internal_error_response())?;
                for entry in entries {
                    let entry = entry.map_err(|_| internal_error_response())?;
                    let path = entry.path();
                    let file_type = entry.file_type().map_err(|_| internal_error_response())?;
                    if file_type.is_dir() {
                        queue.push(path);
                        continue;
                    }
                    if file_type.is_file() {
                        let rel = path
                            .strip_prefix(extract_root)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| path.display().to_string());
                        files.push(rel);
                    }
                }
            }

            files.sort_unstable();
            files.dedup();
            let details = if files.is_empty() {
                "未检测到任何文件".to_string()
            } else {
                format!("检测到文件：{}", files.join(", "))
            };
            Err(config_error_response(&format!(
                "插件包中缺少 plugin.json（{details}）"
            )))
        }
        1 => Ok(candidates.remove(0)),
        _ => Err(config_error_response(
            "插件包中存在多个 plugin.json，请确保仅包含一个插件目录",
        )),
    }
}
