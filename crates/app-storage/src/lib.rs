//! app-storage：存储层（SQLite、迁移、仓储接口）。

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use refinery::Error as MigrationError;
use rusqlite::Connection;
use thiserror::Error;

mod embedded {
    use refinery::embed_migrations;

    embed_migrations!("../../migrations");
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("数据库目录创建失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite 操作失败：{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("迁移执行失败：{0}")]
    Migration(#[from] MigrationError),
    #[error("数据库完整性校验失败：{0}")]
    IntegrityCheck(String),
    #[error("数据库连接锁已中毒")]
    ConnectionPoisoned,
}

pub type StorageResult<T> = Result<T, StorageError>;

/// SQLite 连接封装（MVP 阶段采用单连接 + Mutex）。
#[derive(Debug)]
pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    /// 打开（或创建）指定路径数据库，并执行初始化与迁移。
    pub fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut connection = Connection::open(path)?;
        Self::initialize_connection(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// 打开内存数据库（主要用于测试），并执行初始化与迁移。
    pub fn open_in_memory() -> StorageResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        Self::initialize_connection(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// 获取连接并执行自定义数据库操作。
    pub fn with_connection<T, F>(&self, operation: F) -> StorageResult<T>
    where
        F: FnOnce(&mut Connection) -> StorageResult<T>,
    {
        let mut connection = self.lock_connection()?;
        operation(&mut connection)
    }

    fn initialize_connection(connection: &mut Connection) -> StorageResult<()> {
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch("PRAGMA journal_mode = WAL;")?;
        Self::verify_integrity(connection)?;
        Self::run_migrations(connection)?;
        Ok(())
    }

    fn verify_integrity(connection: &Connection) -> StorageResult<()> {
        let result: String =
            connection.query_row("PRAGMA integrity_check;", [], |row| row.get(0))?;
        if result.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(StorageError::IntegrityCheck(result))
        }
    }

    fn run_migrations(connection: &mut Connection) -> StorageResult<()> {
        embedded::migrations::runner().run(connection)?;
        Ok(())
    }

    fn lock_connection(&self) -> StorageResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StorageError::ConnectionPoisoned)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::Database;
    use super::StorageResult;

    #[test]
    fn open_in_memory_runs_migrations() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let tables = list_tables(&db)?;

        let expected = vec![
            "app_settings",
            "export_tokens",
            "plugins",
            "profile_sources",
            "profiles",
            "refresh_jobs",
            "source_instance_config",
            "source_instances",
            "node_cache",
        ];

        for table in expected {
            assert!(tables.iter().any(|name| name == table), "缺少表：{table}");
        }

        Ok(())
    }

    #[test]
    fn migration_creates_expected_columns() -> StorageResult<()> {
        let db = Database::open_in_memory()?;

        let expected_columns = BTreeMap::from([
            (
                "plugins",
                vec![
                    "id",
                    "plugin_id",
                    "name",
                    "version",
                    "spec_version",
                    "type",
                    "status",
                    "installed_at",
                    "updated_at",
                ],
            ),
            (
                "source_instances",
                vec![
                    "id",
                    "plugin_id",
                    "name",
                    "status",
                    "state_json",
                    "created_at",
                    "updated_at",
                ],
            ),
            (
                "source_instance_config",
                vec!["id", "source_instance_id", "key", "value"],
            ),
            (
                "profiles",
                vec!["id", "name", "description", "created_at", "updated_at"],
            ),
            (
                "profile_sources",
                vec!["profile_id", "source_instance_id", "priority"],
            ),
            (
                "refresh_jobs",
                vec![
                    "id",
                    "source_instance_id",
                    "trigger_type",
                    "status",
                    "started_at",
                    "finished_at",
                    "node_count",
                    "error_code",
                    "error_message",
                ],
            ),
            (
                "export_tokens",
                vec![
                    "id",
                    "profile_id",
                    "token",
                    "token_type",
                    "created_at",
                    "expires_at",
                ],
            ),
            ("app_settings", vec!["key", "value", "updated_at"]),
            (
                "node_cache",
                vec![
                    "id",
                    "source_instance_id",
                    "data_json",
                    "fetched_at",
                    "expires_at",
                ],
            ),
        ]);

        for (table, expected) in expected_columns {
            let columns = list_columns(&db, table)?;
            assert_eq!(columns, expected, "表字段不匹配：{table}");
        }

        Ok(())
    }

    #[test]
    fn opening_database_twice_is_idempotent() -> StorageResult<()> {
        let db_path = unique_test_db_path();

        let first = Database::open(&db_path)?;
        let first_tables = list_tables(&first)?;
        drop(first);

        let second = Database::open(&db_path)?;
        let second_tables = list_tables(&second)?;

        assert_eq!(first_tables, second_tables);

        cleanup_db_files(&db_path);
        Ok(())
    }

    fn list_tables(db: &Database) -> StorageResult<Vec<String>> {
        db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT name \
                 FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )?;

            let names = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(names)
        })
    }

    fn list_columns(db: &Database, table: &str) -> StorageResult<Vec<String>> {
        db.with_connection(|connection| {
            let mut statement =
                connection.prepare("SELECT name FROM pragma_table_info(?) ORDER BY cid")?;

            let names = statement
                .query_map([table], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(names)
        })
    }

    fn unique_test_db_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间异常")
            .as_nanos();
        std::env::temp_dir().join(format!("subforge-app-storage-{nanos}.db"))
    }

    fn cleanup_db_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
    }
}
