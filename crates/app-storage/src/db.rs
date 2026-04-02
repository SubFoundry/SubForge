use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::{StorageError, StorageResult};
mod embedded {
    use refinery::embed_migrations;

    embed_migrations!("../../migrations");
}

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
