use app_common::AppSetting;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::mappers::map_setting_row;
use crate::{Database, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct SettingsRepository<'a> {
    db: &'a Database,
}

impl<'a> SettingsRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn get(&self, key: &str) -> StorageResult<Option<AppSetting>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT key, value, updated_at
                     FROM app_settings
                     WHERE key = ?1
                     LIMIT 1",
                    [key],
                    map_setting_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn set(&self, setting: &AppSetting) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key)
                 DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![setting.key, setting.value, setting.updated_at],
            )?;
            Ok(())
        })
    }

    pub fn get_all(&self) -> StorageResult<Vec<AppSetting>> {
        self.db.with_connection(|connection| {
            let mut statement = connection
                .prepare("SELECT key, value, updated_at FROM app_settings ORDER BY key")?;
            let items = statement
                .query_map([], map_setting_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }
}
