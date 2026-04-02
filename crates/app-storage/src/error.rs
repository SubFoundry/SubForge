use refinery::Error as MigrationError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("数据库目录创建失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite 操作失败：{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON 序列化失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("迁移执行失败：{0}")]
    Migration(#[from] MigrationError),
    #[error("数据库完整性校验失败：{0}")]
    IntegrityCheck(String),
    #[error("数据库连接锁已中毒")]
    ConnectionPoisoned,
}

pub type StorageResult<T> = Result<T, StorageError>;
