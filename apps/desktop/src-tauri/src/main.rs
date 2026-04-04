#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod desktop;

use desktop::{
    CoreManager, apply_main_window_close_behavior, core_api_call, core_events_start,
    core_import_plugin_zip, core_start, core_status, core_stop, desktop_auto_close_gui,
    desktop_get_autostart, desktop_set_autostart, setup_tray,
};
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

fn main() {
    let manager = CoreManager::new().expect("初始化 CoreManager 失败");

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(manager)
        .setup(|app| {
            setup_tray(app.handle())?;
            if should_preload_core_from_env() {
                let manager = app.state::<CoreManager>();
                tauri::async_runtime::block_on(manager.start_core(app.handle()))
                    .map_err(|err| anyhow::anyhow!("预拉起 Core 失败: {err}"))?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    apply_main_window_close_behavior(app_handle).await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            core_start,
            core_stop,
            core_status,
            core_api_call,
            core_import_plugin_zip,
            core_events_start,
            desktop_auto_close_gui,
            desktop_get_autostart,
            desktop_set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("运行 SubForge Desktop 失败");
}

fn should_preload_core_from_env() -> bool {
    let raw = std::env::var("SUBFORGE_DESKTOP_PRELOAD_CORE").unwrap_or_default();
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}
