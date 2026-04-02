use std::collections::BTreeMap;

use rusqlite::params;

use crate::{Database, StorageResult};
#[derive(Debug, Clone, Copy)]
pub struct SourceConfigRepository<'a> {
    db: &'a Database,
}

impl<'a> SourceConfigRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn replace_all(
        &self,
        source_instance_id: &str,
        values: &BTreeMap<String, String>,
    ) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            let tx = connection.transaction()?;
            tx.execute(
                "DELETE FROM source_instance_config WHERE source_instance_id = ?1",
                [source_instance_id],
            )?;
            for (key, value) in values {
                tx.execute(
                    "INSERT INTO source_instance_config (id, source_instance_id, key, value)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        format!("{source_instance_id}:{key}"),
                        source_instance_id,
                        key,
                        value
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_all(&self, source_instance_id: &str) -> StorageResult<BTreeMap<String, String>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT key, value
                 FROM source_instance_config
                 WHERE source_instance_id = ?1
                 ORDER BY key",
            )?;
            let rows = statement.query_map([source_instance_id], |row| {
                let key: String = row.get("key")?;
                let value: String = row.get("value")?;
                Ok((key, value))
            })?;

            let mut values = BTreeMap::new();
            for row in rows {
                let (key, value) = row?;
                values.insert(key, value);
            }
            Ok(values)
        })
    }

    pub fn delete_all(&self, source_instance_id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "DELETE FROM source_instance_config
                 WHERE source_instance_id = ?1",
                [source_instance_id],
            )?;
            Ok(affected)
        })
    }
}
