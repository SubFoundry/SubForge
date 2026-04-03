use tauri::{AppHandle, State};

use super::core_manager::CoreManager;
use super::types::{CoreApiRequest, CoreApiResponse, CoreStatusPayload, PluginImportRequest};
use super::window_lifecycle::close_main_window;

#[tauri::command]
pub(crate) async fn core_start(
    manager: State<'_, CoreManager>,
) -> Result<CoreStatusPayload, String> {
    manager.start_core().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn core_stop(
    manager: State<'_, CoreManager>,
) -> Result<CoreStatusPayload, String> {
    manager.stop_core().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn core_status(
    manager: State<'_, CoreManager>,
) -> Result<CoreStatusPayload, String> {
    manager
        .compose_status_payload()
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn core_api_call(
    manager: State<'_, CoreManager>,
    request: CoreApiRequest,
) -> Result<CoreApiResponse, String> {
    manager
        .proxy_api_call(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn core_import_plugin_zip(
    manager: State<'_, CoreManager>,
    request: PluginImportRequest,
) -> Result<CoreApiResponse, String> {
    manager
        .import_plugin_zip(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn core_events_start(
    manager: State<'_, CoreManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    manager
        .start_events_bridge(app_handle)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn desktop_auto_close_gui(app_handle: AppHandle) -> Result<(), String> {
    close_main_window(app_handle);
    Ok(())
}
