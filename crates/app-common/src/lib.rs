//! app-common：公共模型与错误定义。

mod error;
mod models;
mod plugin;
#[cfg(test)]
mod tests;

pub use error::{AppError, AppResult, ErrorResponse};
pub use models::{
    AppSetting, ClashRoutingTemplate, ClashRoutingTemplateGroup, Plugin, Profile, ProfileSource,
    ProxyNode, ProxyProtocol, ProxyTransport, RoutingTemplateGroupIr, RoutingTemplateIr,
    RoutingTemplateSourceKernel, SourceInstance, TlsConfig,
};
pub use plugin::{
    ConfigSchema, ConfigSchemaProperty, ConfigSchemaUi, PluginEntrypoints, PluginManifest,
    PluginType,
};
