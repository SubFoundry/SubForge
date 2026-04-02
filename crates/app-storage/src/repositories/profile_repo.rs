use app_common::Profile;
use rusqlite::{OptionalExtension, params};

use crate::mappers::map_profile_row;
use crate::{Database, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct ProfileRepository<'a> {
    db: &'a Database,
}

impl<'a> ProfileRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, profile: &Profile) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO profiles
                 (id, name, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile.id,
                    profile.name,
                    profile.description,
                    profile.created_at,
                    profile.updated_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<Profile>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, name, description, created_at, updated_at
                     FROM profiles
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_profile_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list(&self) -> StorageResult<Vec<Profile>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, created_at, updated_at
                 FROM profiles
                 ORDER BY created_at, id",
            )?;
            let items = statement
                .query_map([], map_profile_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn update(&self, profile: &Profile) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE profiles
                 SET name = ?1, description = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![
                    profile.name,
                    profile.description,
                    profile.updated_at,
                    profile.id
                ],
            )?;
            Ok(affected)
        })
    }

    pub fn delete(&self, id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
            Ok(affected)
        })
    }

    pub fn add_source(
        &self,
        profile_id: &str,
        source_instance_id: &str,
        priority: i64,
    ) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO profile_sources (profile_id, source_instance_id, priority)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(profile_id, source_instance_id)
                 DO UPDATE SET priority = excluded.priority",
                params![profile_id, source_instance_id, priority],
            )?;
            Ok(())
        })
    }

    pub fn remove_source(
        &self,
        profile_id: &str,
        source_instance_id: &str,
    ) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "DELETE FROM profile_sources
                 WHERE profile_id = ?1 AND source_instance_id = ?2",
                params![profile_id, source_instance_id],
            )?;
            Ok(affected)
        })
    }
}
