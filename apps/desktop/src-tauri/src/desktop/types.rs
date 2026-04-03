use std::collections::BTreeMap;
use std::process::Child;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;

pub(super) const DEFAULT_CORE_HOST: &str = "127.0.0.1";
pub(super) const DEFAULT_CORE_PORT: u16 = 18118;
pub(super) const MAX_PLUGIN_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
pub(super) const SETTING_KEY_GUI_CLOSE_BEHAVIOR: &str = "gui_close_behavior";
pub(super) const SETTING_KEY_TRAY_MINIMIZE: &str = "tray_minimize";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiCloseBehavior {
    TrayMinimize,
    CloseGui,
    CloseGuiAndStopCore,
}

impl GuiCloseBehavior {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tray_minimize" => Some(Self::TrayMinimize),
            "close_gui" => Some(Self::CloseGui),
            "close_gui_and_stop_core" => Some(Self::CloseGuiAndStopCore),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoreStatusPayload {
    pub(super) running: bool,
    pub(super) base_url: String,
    pub(super) version: Option<String>,
    pub(super) pid: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoreApiRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) body: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginImportRequest {
    pub(super) file_name: String,
    pub(super) payload_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoreApiResponse {
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SettingsResponse {
    pub(super) settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CoreBootstrapLine {
    pub(super) version: String,
    pub(super) listen_addr: String,
    pub(super) port: u16,
    pub(super) admin_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CoreEventPayload {
    pub(super) event: String,
    pub(super) message: String,
    pub(super) source_id: Option<String>,
    pub(super) timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CoreBridgeEvent {
    pub(super) kind: String,
    pub(super) payload: Option<CoreEventPayload>,
    pub(super) message: Option<String>,
}

#[derive(Debug)]
pub(super) struct CoreState {
    pub(super) child: Option<Child>,
    pub(super) admin_token: Option<String>,
    pub(super) base_url: String,
    pub(super) version: Option<String>,
    pub(super) pid: Option<u32>,
    pub(super) events_task: Option<JoinHandle<()>>,
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            child: None,
            admin_token: None,
            base_url: format!("http://{DEFAULT_CORE_HOST}:{DEFAULT_CORE_PORT}"),
            version: None,
            pid: None,
            events_task: None,
        }
    }
}
