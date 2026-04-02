use mlua::Error as LuaError;

use crate::PluginRuntimeError;

use super::{
    HOOK_LIMIT_SENTINEL, HOOK_TIMEOUT_SENTINEL, HTTP_REQUEST_LIMIT_SENTINEL,
    HTTP_RESPONSE_LIMIT_SENTINEL, SCRIPT_HTTP_MAX_REQUESTS, SCRIPT_HTTP_MAX_RESPONSE_BYTES,
};

pub(super) fn map_lua_error(error: LuaError) -> PluginRuntimeError {
    if runtime_message_contains(&error, HOOK_TIMEOUT_SENTINEL) {
        return PluginRuntimeError::ScriptTimeout("脚本执行超过超时上限".to_string());
    }

    if runtime_message_contains(&error, HOOK_LIMIT_SENTINEL) {
        return PluginRuntimeError::ScriptLimit("脚本指令数超过上限".to_string());
    }

    if runtime_message_contains(&error, HTTP_REQUEST_LIMIT_SENTINEL) {
        return PluginRuntimeError::ScriptLimit(format!(
            "http.request 次数超过上限：{}",
            SCRIPT_HTTP_MAX_REQUESTS
        ));
    }

    if runtime_message_contains(&error, HTTP_RESPONSE_LIMIT_SENTINEL) {
        return PluginRuntimeError::ScriptLimit(format!(
            "http.request 响应体超过上限：{} bytes",
            SCRIPT_HTTP_MAX_RESPONSE_BYTES
        ));
    }

    if let Some(message) = memory_error_message(&error) {
        return PluginRuntimeError::ScriptLimit(format!("脚本内存超过上限：{message}"));
    }

    PluginRuntimeError::ScriptRuntime(error.to_string())
}

pub(super) fn map_secret_error(action: &str, error: app_secrets::SecretError) -> LuaError {
    LuaError::runtime(format!("{action} 失败（{}）：{error}", error.code()))
}

fn runtime_message_contains(error: &LuaError, marker: &str) -> bool {
    match error {
        LuaError::RuntimeError(message) => message.contains(marker),
        LuaError::CallbackError { cause, .. }
        | LuaError::WithContext { cause, .. }
        | LuaError::BadArgument { cause, .. } => runtime_message_contains(cause.as_ref(), marker),
        _ => false,
    }
}

fn memory_error_message(error: &LuaError) -> Option<&str> {
    match error {
        LuaError::MemoryError(message) => Some(message.as_str()),
        LuaError::CallbackError { cause, .. }
        | LuaError::WithContext { cause, .. }
        | LuaError::BadArgument { cause, .. } => memory_error_message(cause.as_ref()),
        _ => None,
    }
}
