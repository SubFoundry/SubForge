use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use app_secrets::{MemorySecretStore, SecretStore};
use mlua::{
    Error as LuaError, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue,
    Value as LuaValue, VmState,
};
use serde_json::Value;

use crate::{PluginRuntimeError, PluginRuntimeResult};

mod error_map;
mod runtime_apis;
#[cfg(test)]
mod tests;

use error_map::map_lua_error;
use runtime_apis::{new_cookie_store, register_runtime_apis};

const DEFAULT_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const DEFAULT_HOOK_STEP: u32 = 1000;
const DEFAULT_MAX_INSTRUCTIONS: u64 = 100_000_000;
const DEFAULT_NETWORK_PROFILE: &str = "standard";
const DEFAULT_PLUGIN_ID: &str = "runtime.default";
const SUPPORTED_RUNTIME_CAPABILITIES: &[&str] = &[
    "http", "cookie", "json", "html", "base64", "secret", "log", "time",
];
const SCRIPT_HTTP_TIMEOUT_MS: u64 = 15_000;
const SCRIPT_HTTP_MAX_REQUESTS: usize = 20;
const SCRIPT_HTTP_MAX_REDIRECTS: usize = 5;
const SCRIPT_HTTP_MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const HOOK_TIMEOUT_SENTINEL: &str = "__subforge_script_timeout__";
const HOOK_LIMIT_SENTINEL: &str = "__subforge_script_limit__";
const HTTP_REQUEST_LIMIT_SENTINEL: &str = "__subforge_http_request_limit__";
const HTTP_RESPONSE_LIMIT_SENTINEL: &str = "__subforge_http_response_limit__";
const LOG_PREFIX: &str = "subforge.lua";

const DISABLED_GLOBALS: &[&str] = &[
    "os",
    "io",
    "debug",
    "loadfile",
    "dofile",
    "require",
    "rawget",
    "rawset",
    "collectgarbage",
    "package",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLogLevel {
    Info,
    Warn,
    Error,
}

impl RuntimeLogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

pub trait RuntimeLogSink: Send + Sync {
    fn emit(&self, level: RuntimeLogLevel, message: &str);
}

#[derive(Clone)]
pub struct LuaSandboxConfig {
    pub memory_limit_bytes: usize,
    pub timeout: Duration,
    pub max_instructions: u64,
    pub instruction_hook_step: u32,
    pub network_profile: String,
    pub plugin_id: String,
    pub capabilities: Vec<String>,
    pub secret_store: Arc<dyn SecretStore>,
    pub log_sink: Option<Arc<dyn RuntimeLogSink>>,
}

impl std::fmt::Debug for LuaSandboxConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaSandboxConfig")
            .field("memory_limit_bytes", &self.memory_limit_bytes)
            .field("timeout", &self.timeout)
            .field("max_instructions", &self.max_instructions)
            .field("instruction_hook_step", &self.instruction_hook_step)
            .field("network_profile", &self.network_profile)
            .field("plugin_id", &self.plugin_id)
            .field("capabilities", &self.capabilities)
            .field("secret_store", &"<secret-store>")
            .field(
                "log_sink",
                &self.log_sink.as_ref().map(|_| "<runtime-log-sink>"),
            )
            .finish()
    }
}

impl Default for LuaSandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            max_instructions: DEFAULT_MAX_INSTRUCTIONS,
            instruction_hook_step: DEFAULT_HOOK_STEP,
            network_profile: DEFAULT_NETWORK_PROFILE.to_string(),
            plugin_id: DEFAULT_PLUGIN_ID.to_string(),
            capabilities: SUPPORTED_RUNTIME_CAPABILITIES
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
            secret_store: Arc::new(MemorySecretStore::new()),
            log_sink: None,
        }
    }
}

impl LuaSandboxConfig {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_memory_limit_bytes(mut self, memory_limit_bytes: usize) -> Self {
        self.memory_limit_bytes = memory_limit_bytes;
        self
    }

    pub fn with_instruction_limit(
        mut self,
        max_instructions: u64,
        instruction_hook_step: u32,
    ) -> Self {
        self.max_instructions = max_instructions;
        self.instruction_hook_step = instruction_hook_step.max(1);
        self
    }

    pub fn with_network_profile(mut self, profile: impl Into<String>) -> Self {
        self.network_profile = profile.into();
        self
    }

    pub fn with_plugin_id(mut self, plugin_id: impl Into<String>) -> Self {
        self.plugin_id = plugin_id.into();
        self
    }

    pub fn with_capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.capabilities = capabilities.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_secret_store(mut self, secret_store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = secret_store;
        self
    }

    pub fn with_log_sink(mut self, log_sink: Arc<dyn RuntimeLogSink>) -> Self {
        self.log_sink = Some(log_sink);
        self
    }
}

pub struct LuaSandbox {
    lua: Lua,
    config: LuaSandboxConfig,
    http_request_counter: Arc<AtomicUsize>,
}

impl LuaSandbox {
    pub fn new() -> PluginRuntimeResult<Self> {
        Self::new_with_config(LuaSandboxConfig::default())
    }

    pub fn new_with_config(config: LuaSandboxConfig) -> PluginRuntimeResult<Self> {
        if config.plugin_id.trim().is_empty() {
            return Err(PluginRuntimeError::ScriptRuntime(
                "plugin_id 不能为空".to_string(),
            ));
        }

        let unsupported_capability = config
            .capabilities
            .iter()
            .find(|capability| !SUPPORTED_RUNTIME_CAPABILITIES.contains(&capability.as_str()));
        if let Some(capability) = unsupported_capability {
            return Err(PluginRuntimeError::ScriptRuntime(format!(
                "不支持的 runtime capability：{capability}"
            )));
        }

        let secret_scope = format!("plugin:{}", config.plugin_id);
        config
            .secret_store
            .list_keys(&secret_scope)
            .map_err(|error| {
                PluginRuntimeError::ScriptRuntime(format!(
                    "secret 命名空间初始化失败（{secret_scope}）：{error}"
                ))
            })?;

        let lua =
            Lua::new_with(mlua::StdLib::ALL_SAFE, LuaOptions::default()).map_err(map_lua_error)?;
        lua.set_memory_limit(config.memory_limit_bytes)
            .map_err(map_lua_error)?;
        let cookie_store = new_cookie_store();
        let http_request_counter = Arc::new(AtomicUsize::new(0));
        disable_globals(&lua)?;
        register_runtime_apis(
            &lua,
            &config,
            Arc::clone(&cookie_store),
            secret_scope,
            Arc::clone(&http_request_counter),
            config.log_sink.clone(),
        )?;

        Ok(Self {
            lua,
            config,
            http_request_counter,
        })
    }

    pub fn exec_file(
        &self,
        path: impl AsRef<Path>,
        entry_fn: &str,
        args: &[Value],
    ) -> PluginRuntimeResult<Value> {
        let script_path = path.as_ref();
        let script_content = fs::read_to_string(script_path)?;
        self.http_request_counter.store(0, AtomicOrdering::Relaxed);
        self.install_limits_hook()?;

        let execution_result = (|| -> PluginRuntimeResult<Value> {
            let chunk_name = script_path.display().to_string();
            self.lua
                .load(&script_content)
                .set_name(chunk_name)
                .exec()
                .map_err(map_lua_error)?;

            let globals = self.lua.globals();
            let entrypoint: Function = globals.get(entry_fn).map_err(map_lua_error)?;
            let lua_args = pack_args(&self.lua, args)?;
            let lua_result: LuaValue = entrypoint.call(lua_args).map_err(map_lua_error)?;
            self.lua.from_value(lua_result).map_err(map_lua_error)
        })();

        self.lua.remove_hook();
        execution_result
    }

    fn install_limits_hook(&self) -> PluginRuntimeResult<()> {
        let started = Instant::now();
        let timeout = self.config.timeout;
        let max_instructions = self.config.max_instructions;
        let instruction_step = self.config.instruction_hook_step as u64;
        let executed_instructions = Arc::new(AtomicU64::new(0));
        let instruction_counter = Arc::clone(&executed_instructions);

        self.lua
            .set_hook(
                HookTriggers::new().every_nth_instruction(self.config.instruction_hook_step),
                move |_lua, _debug| {
                    if started.elapsed() >= timeout {
                        return Err(LuaError::runtime(HOOK_TIMEOUT_SENTINEL));
                    }

                    let next = instruction_counter
                        .fetch_add(instruction_step, Ordering::Relaxed)
                        .saturating_add(instruction_step);
                    if next > max_instructions {
                        return Err(LuaError::runtime(HOOK_LIMIT_SENTINEL));
                    }

                    Ok(VmState::Continue)
                },
            )
            .map_err(map_lua_error)
    }
}

fn disable_globals(lua: &Lua) -> PluginRuntimeResult<()> {
    let globals = lua.globals();
    for name in DISABLED_GLOBALS {
        globals.raw_remove(*name).map_err(map_lua_error)?;
    }
    Ok(())
}

fn pack_args(lua: &Lua, args: &[Value]) -> PluginRuntimeResult<MultiValue> {
    let mut lua_values = Vec::with_capacity(args.len());
    for arg in args {
        let value = lua.to_value(arg).map_err(map_lua_error)?;
        lua_values.push(value);
    }
    Ok(MultiValue::from_vec(lua_values))
}
