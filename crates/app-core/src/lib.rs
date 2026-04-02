//! app-core：业务编排层（调度、刷新、重试、状态机）。

mod engine;
mod error;
mod fetcher;
mod parser;
mod plugin_install;
mod script_executor;
mod source_service;
mod utils;

pub use engine::{Engine, SourceRefreshResult};
pub use error::{CoreError, CoreResult};
pub use fetcher::StaticFetcher;
pub use parser::{SubscriptionParser, UriListParser};
pub use plugin_install::PluginInstallService;
pub use source_service::SourceService;

use std::collections::BTreeMap;

use app_common::SourceInstance;
use serde_json::Value;

const SECRET_PLACEHOLDER: &str = "••••••";

#[derive(Debug, Clone, PartialEq)]
pub struct SourceWithConfig {
    pub source: SourceInstance,
    pub config: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests;
