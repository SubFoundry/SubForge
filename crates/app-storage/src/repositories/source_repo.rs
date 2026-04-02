use app_common::SourceInstance;
use rusqlite::{OptionalExtension, params};

use crate::mappers::map_source_row;
use crate::{Database, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct SourceRepository<'a> {
    db: &'a Database,
}

impl<'a> SourceRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, source: &SourceInstance) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO source_instances
                 (id, plugin_id, name, status, state_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    source.id,
                    source.plugin_id,
                    source.name,
                    source.status,
                    source.state_json,
                    source.created_at,
                    source.updated_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<SourceInstance>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                     FROM source_instances
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_source_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list(&self) -> StorageResult<Vec<SourceInstance>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                 FROM source_instances
                 ORDER BY created_at, id",
            )?;
            let items = statement
                .query_map([], map_source_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn list_by_plugin(&self, plugin_id: &str) -> StorageResult<Vec<SourceInstance>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                 FROM source_instances
                 WHERE plugin_id = ?1
                 ORDER BY created_at, id",
            )?;
            let items = statement
                .query_map([plugin_id], map_source_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn update(&self, source: &SourceInstance) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE source_instances
                 SET plugin_id = ?1, name = ?2, status = ?3, state_json = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![
                    source.plugin_id,
                    source.name,
                    source.status,
                    source.state_json,
                    source.updated_at,
                    source.id
                ],
            )?;
            Ok(affected)
        })
    }

    pub fn delete(&self, id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected =
                connection.execute("DELETE FROM source_instances WHERE id = ?1", [id])?;
            Ok(affected)
        })
    }
}
