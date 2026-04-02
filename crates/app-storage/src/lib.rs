//! app-storage：存储层（SQLite、迁移、仓储接口）。

mod db;
mod error;
mod mappers;
mod models;
mod repositories;

#[cfg(test)]
mod tests;

pub use db::Database;
pub use error::{StorageError, StorageResult};
pub use models::{ExportToken, NodeCacheEntry, RefreshJob};
pub use repositories::{
    ExportTokenRepository, NodeCacheRepository, PluginRepository, ProfileRepository,
    RefreshJobRepository, SettingsRepository, SourceConfigRepository, SourceRepository,
};
