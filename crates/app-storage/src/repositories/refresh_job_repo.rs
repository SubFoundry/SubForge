use rusqlite::{OptionalExtension, params};

use crate::mappers::map_refresh_job_row;
use crate::{Database, RefreshJob, StorageError, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct RefreshJobRepository<'a> {
    db: &'a Database,
}

impl<'a> RefreshJobRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, job: &RefreshJob) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO refresh_jobs
                 (id, source_instance_id, trigger_type, status, started_at, finished_at, node_count, error_code, error_message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    job.id,
                    job.source_instance_id,
                    job.trigger_type,
                    job.status,
                    job.started_at,
                    job.finished_at,
                    job.node_count,
                    job.error_code,
                    job.error_message
                ],
            )?;
            Ok(())
        })
    }

    pub fn mark_success(
        &self,
        id: &str,
        finished_at: &str,
        node_count: i64,
    ) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE refresh_jobs
                 SET status = ?1,
                     finished_at = ?2,
                     node_count = ?3,
                     error_code = NULL,
                     error_message = NULL
                 WHERE id = ?4",
                params!["success", finished_at, node_count, id],
            )?;
            Ok(affected)
        })
    }

    pub fn mark_failed(
        &self,
        id: &str,
        finished_at: &str,
        error_code: &str,
        error_message: &str,
    ) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE refresh_jobs
                 SET status = ?1,
                     finished_at = ?2,
                     node_count = NULL,
                     error_code = ?3,
                     error_message = ?4
                 WHERE id = ?5",
                params!["failed", finished_at, error_code, error_message, id],
            )?;
            Ok(affected)
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<RefreshJob>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, source_instance_id, trigger_type, status, started_at, finished_at, node_count, error_code, error_message
                     FROM refresh_jobs
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_refresh_job_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list_by_source(&self, source_instance_id: &str) -> StorageResult<Vec<RefreshJob>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, source_instance_id, trigger_type, status, started_at, finished_at, node_count, error_code, error_message
                 FROM refresh_jobs
                 WHERE source_instance_id = ?1
                 ORDER BY started_at, id",
            )?;
            let items = statement
                .query_map([source_instance_id], map_refresh_job_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }
}
