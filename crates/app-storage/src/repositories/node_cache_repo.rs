use app_common::ProxyNode;
use rusqlite::{OptionalExtension, params};

use crate::{Database, NodeCacheEntry, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct NodeCacheRepository<'a> {
    db: &'a Database,
}

impl<'a> NodeCacheRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert_nodes(
        &self,
        source_instance_id: &str,
        nodes: &[ProxyNode],
        fetched_at: &str,
        expires_at: Option<&str>,
    ) -> StorageResult<()> {
        let cache_id = format!("node_cache:{source_instance_id}");
        let data_json = serde_json::to_string(nodes)?;

        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO node_cache (id, source_instance_id, data_json, fetched_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id)
                 DO UPDATE SET data_json = excluded.data_json,
                               fetched_at = excluded.fetched_at,
                               expires_at = excluded.expires_at",
                params![
                    cache_id,
                    source_instance_id,
                    data_json,
                    fetched_at,
                    expires_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_source(&self, source_instance_id: &str) -> StorageResult<Option<NodeCacheEntry>> {
        self.db.with_connection(|connection| {
            let raw = connection
                .query_row(
                    "SELECT id, source_instance_id, data_json, fetched_at, expires_at
                     FROM node_cache
                     WHERE source_instance_id = ?1
                     LIMIT 1",
                    [source_instance_id],
                    |row| {
                        Ok((
                            row.get::<_, String>("id")?,
                            row.get::<_, String>("source_instance_id")?,
                            row.get::<_, String>("data_json")?,
                            row.get::<_, String>("fetched_at")?,
                            row.get::<_, Option<String>>("expires_at")?,
                        ))
                    },
                )
                .optional()?;

            if let Some((id, source_instance_id, data_json, fetched_at, expires_at)) = raw {
                let nodes = serde_json::from_str::<Vec<ProxyNode>>(&data_json)?;
                Ok(Some(NodeCacheEntry {
                    id,
                    source_instance_id,
                    nodes,
                    fetched_at,
                    expires_at,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn delete_by_source(&self, source_instance_id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "DELETE FROM node_cache
                 WHERE source_instance_id = ?1",
                [source_instance_id],
            )?;
            Ok(affected)
        })
    }
}
