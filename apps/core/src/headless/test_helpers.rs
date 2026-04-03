use anyhow::Result;
use app_storage::Database;

pub(crate) fn list_profile_source_ids(
    database: &Database,
    profile_id: &str,
) -> Result<Vec<String>> {
    database
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT source_instance_id
                 FROM profile_sources
                 WHERE profile_id = ?1
                 ORDER BY priority, source_instance_id",
            )?;
            let rows = statement.query_map([profile_id], |row| row.get::<_, String>(0))?;
            let mut source_ids = Vec::new();
            for row in rows {
                source_ids.push(row?);
            }
            Ok(source_ids)
        })
        .map_err(Into::into)
}

pub(crate) fn list_sources(database: &Database) -> Result<Vec<app_common::SourceInstance>> {
    app_storage::SourceRepository::new(database)
        .list()
        .map_err(Into::into)
}
