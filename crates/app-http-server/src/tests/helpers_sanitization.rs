use app_core::CoreError;
use app_plugin_runtime::PluginRuntimeError;
use app_storage::StorageError;
use axum::Json;

use crate::helpers::{core_error_to_response, storage_error_to_response};

#[test]
fn storage_error_response_does_not_expose_internal_details() {
    let io_error = std::io::Error::other("open C:\\secret\\subforge.db failed");
    let (_, Json(payload)) = storage_error_to_response(StorageError::Io(io_error));
    assert_eq!(payload.code, "E_INTERNAL");
    assert_eq!(payload.message, "Internal server error");
}

#[test]
fn core_internal_error_response_does_not_expose_internal_details() {
    let io_error = std::io::Error::other("permission denied: /var/lib/subforge/subforge.db");
    let (_, Json(payload)) = core_error_to_response(CoreError::Storage(StorageError::Io(io_error)));
    assert_eq!(payload.code, "E_INTERNAL");
    assert_eq!(payload.message, "Internal server error");
}

#[test]
fn core_script_runtime_error_keeps_runtime_message() {
    let (_, Json(payload)) = core_error_to_response(CoreError::PluginRuntime(
        PluginRuntimeError::ScriptRuntime("script failed".to_string()),
    ));
    assert_eq!(payload.code, "E_SCRIPT_RUNTIME");
    assert_eq!(payload.message, "script failed");
}

#[test]
fn core_plugin_invalid_error_maps_to_bad_request_message() {
    let (status, Json(payload)) = core_error_to_response(CoreError::PluginRuntime(
        PluginRuntimeError::Invalid("缺少入口脚本 fetch".to_string()),
    ));
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(payload.code, "E_PLUGIN_INVALID");
    assert_eq!(payload.message, "缺少入口脚本 fetch");
}
