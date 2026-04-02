use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginRuntimeError {
    #[error("读取插件文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("plugin.json 解析失败：{0}")]
    ManifestParse(#[source] serde_json::Error),
    #[error("schema.json 解析失败：{0}")]
    SchemaParse(#[source] serde_json::Error),
    #[error("插件清单非法：{0}")]
    Invalid(String),
    #[error("插件与平台不兼容：{0}")]
    Incompatible(String),
    #[error("脚本执行超时：{0}")]
    ScriptTimeout(String),
    #[error("脚本资源超限：{0}")]
    ScriptLimit(String),
    #[error("脚本运行失败：{0}")]
    ScriptRuntime(String),
}

impl PluginRuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Incompatible(_) => "E_PLUGIN_INCOMPATIBLE",
            Self::ScriptTimeout(_) => "E_SCRIPT_TIMEOUT",
            Self::ScriptLimit(_) => "E_SCRIPT_LIMIT",
            Self::ScriptRuntime(_) => "E_SCRIPT_RUNTIME",
            _ => "E_PLUGIN_INVALID",
        }
    }
}

pub type PluginRuntimeResult<T> = Result<T, PluginRuntimeError>;
