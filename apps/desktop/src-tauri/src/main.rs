mod desktop;

use desktop::{
    CoreManager, apply_main_window_close_behavior, core_api_call, core_events_start,
    core_import_plugin_zip, core_start, core_status, core_stop, desktop_auto_close_gui, setup_tray,
};
use tauri::Manager;

fn main() {
    let manager = CoreManager::new().expect("初始化 CoreManager 失败");

    tauri::Builder::default()
        .manage(manager)
        .setup(|app| {
            setup_tray(&app.handle())?;
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
            desktop_auto_close_gui
        ])
        .run(tauri::generate_context!())
        .expect("运行 SubForge Desktop 失败");
}
