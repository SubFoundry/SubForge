//! app-common：公共模型与错误定义。

mod error;
mod models;
mod plugin;
#[cfg(test)]
mod tests;

pub use error::{AppError, AppResult, ErrorResponse};
pub use models::{
    AppSetting, Plugin, Profile, ProfileSource, ProxyNode, ProxyProtocol, ProxyTransport,
    SourceInstance, TlsConfig,
};
pub use plugin::{
    ConfigSchema, ConfigSchemaProperty, ConfigSchemaUi, PluginEntrypoints, PluginManifest,
    PluginType,
};
