use std::fs;
use std::path::PathBuf;

use app_plugin_runtime::LoadedPlugin;

use crate::CoreResult;
use crate::script_executor::errors::script_runtime_error;

pub(super) fn resolve_entrypoint_path(
    loaded_plugin: &LoadedPlugin,
    entrypoint: &str,
    stage_name: &str,
) -> CoreResult<PathBuf> {
    let entrypoint = entrypoint.trim();
    if entrypoint.is_empty() {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口路径不能为空"
        )));
    }

    let raw_path = loaded_plugin.root_dir.join(entrypoint);
    let canonical = fs::canonicalize(&raw_path).map_err(|error| {
        script_runtime_error(&format!(
            "{stage_name} 入口脚本不存在或不可访问（{}）：{error}",
            raw_path.display()
        ))
    })?;
    if !canonical.starts_with(&loaded_plugin.root_dir) {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口脚本路径越界：{}",
            canonical.display()
        )));
    }
    if !canonical.is_file() {
        return Err(script_runtime_error(&format!(
            "{stage_name} 入口脚本不是文件：{}",
            canonical.display()
        )));
    }

    Ok(canonical)
}
