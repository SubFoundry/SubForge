use app_common::Plugin;
use rusqlite::{OptionalExtension, params};

use crate::mappers::map_plugin_row;
use crate::{Database, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct PluginRepository<'a> {
    db: &'a Database,
}

impl<'a> PluginRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, plugin: &Plugin) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO plugins
                 (id, plugin_id, name, version, spec_version, type, status, installed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    plugin.id,
                    plugin.plugin_id,
                    plugin.name,
                    plugin.version,
                    plugin.spec_version,
                    plugin.plugin_type,
                    plugin.status,
                    plugin.installed_at,
                    plugin.updated_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<Plugin>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                     FROM plugins
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_plugin_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn get_by_plugin_id(&self, plugin_id: &str) -> StorageResult<Option<Plugin>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                     FROM plugins
                     WHERE plugin_id = ?1
                     LIMIT 1",
                    [plugin_id],
                    map_plugin_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list(&self) -> StorageResult<Vec<Plugin>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                 FROM plugins
                 ORDER BY installed_at, id",
            )?;
            let items = statement
                .query_map([], map_plugin_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn update_status(&self, id: &str, status: &str, updated_at: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE plugins
                 SET status = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![status, updated_at, id],
            )?;
            Ok(affected)
        })
    }

    pub fn delete(&self, id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute("DELETE FROM plugins WHERE id = ?1", [id])?;
            Ok(affected)
        })
    }
}
