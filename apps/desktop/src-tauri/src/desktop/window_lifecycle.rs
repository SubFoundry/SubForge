use tauri::{AppHandle, Manager};

use super::core_manager::CoreManager;
use super::types::GuiCloseBehavior;

pub(crate) async fn apply_main_window_close_behavior(app_handle: AppHandle) {
    let manager = app_handle.state::<CoreManager>();
    let behavior = manager.resolve_gui_close_behavior().await;

    let Some(window) = app_handle.get_webview_window("main") else {
        return;
    };

    match behavior {
        GuiCloseBehavior::TrayMinimize => {
            if window.minimize().is_err() {
                let _ = window.hide();
            }
        }
        GuiCloseBehavior::CloseGui => {
            close_main_window(app_handle);
        }
        GuiCloseBehavior::CloseGuiAndStopCore => {
            let _ = manager.stop_core().await;
            close_main_window(app_handle);
        }
    }
}

pub(crate) fn restore_main_window(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub(crate) fn close_main_window(app_handle: AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.destroy();
    }
}
