use rusqlite::{OptionalExtension, params};

use crate::mappers::map_export_token_row;
use crate::{Database, ExportToken, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct ExportTokenRepository<'a> {
    db: &'a Database,
}

impl<'a> ExportTokenRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, token: &ExportToken) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO export_tokens (id, profile_id, token, token_type, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    token.id,
                    token.profile_id,
                    token.token,
                    token.token_type,
                    token.created_at,
                    token.expires_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_active_token(&self, profile_id: &str) -> StorageResult<Option<ExportToken>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, profile_id, token, token_type, created_at, expires_at
                     FROM export_tokens
                     WHERE profile_id = ?1 AND expires_at IS NULL
                     ORDER BY created_at DESC, id DESC
                     LIMIT 1",
                    [profile_id],
                    map_export_token_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn is_valid_token(
        &self,
        profile_id: &str,
        token: &str,
        now_rfc3339: &str,
    ) -> StorageResult<bool> {
        self.db.with_connection(|connection| {
            let exists = connection
                .query_row(
                    "SELECT 1
                     FROM export_tokens
                     WHERE profile_id = ?1
                       AND token = ?2
                       AND (expires_at IS NULL OR expires_at > ?3)
                     LIMIT 1",
                    params![profile_id, token, now_rfc3339],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            Ok(exists)
        })
    }
}
