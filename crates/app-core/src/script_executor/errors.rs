use app_plugin_runtime::PluginRuntimeError;

use crate::CoreError;

pub(super) fn script_runtime_error(message: &str) -> CoreError {
    CoreError::PluginRuntime(PluginRuntimeError::ScriptRuntime(message.to_string()))
}
