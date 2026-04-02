//! app-storage：存储层（SQLite、迁移、仓储接口）。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use app_common::{AppSetting, Plugin, Profile, ProxyNode, SourceInstance};
use refinery::Error as MigrationError;
use rusqlite::{Connection, OptionalExtension, Row, params};
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

#[derive(Debug, Clone, Copy)]
pub struct PluginRepository<'a> {
    db: &'a Database,
}

impl<'a> PluginRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, plugin: &Plugin) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO plugins
                 (id, plugin_id, name, version, spec_version, type, status, installed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    plugin.id,
                    plugin.plugin_id,
                    plugin.name,
                    plugin.version,
                    plugin.spec_version,
                    plugin.plugin_type,
                    plugin.status,
                    plugin.installed_at,
                    plugin.updated_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<Plugin>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                     FROM plugins
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_plugin_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn get_by_plugin_id(&self, plugin_id: &str) -> StorageResult<Option<Plugin>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                     FROM plugins
                     WHERE plugin_id = ?1
                     LIMIT 1",
                    [plugin_id],
                    map_plugin_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list(&self) -> StorageResult<Vec<Plugin>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, version, spec_version, type AS plugin_type, status, installed_at, updated_at
                 FROM plugins
                 ORDER BY installed_at, id",
            )?;
            let items = statement
                .query_map([], map_plugin_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn update_status(&self, id: &str, status: &str, updated_at: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE plugins
                 SET status = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![status, updated_at, id],
            )?;
            Ok(affected)
        })
    }

    pub fn delete(&self, id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute("DELETE FROM plugins WHERE id = ?1", [id])?;
            Ok(affected)
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SourceRepository<'a> {
    db: &'a Database,
}

impl<'a> SourceRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn insert(&self, source: &SourceInstance) -> StorageResult<()> {
        self.db.with_connection(|connection| {
            connection.execute(
                "INSERT INTO source_instances
                 (id, plugin_id, name, status, state_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    source.id,
                    source.plugin_id,
                    source.name,
                    source.status,
                    source.state_json,
                    source.created_at,
                    source.updated_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_by_id(&self, id: &str) -> StorageResult<Option<SourceInstance>> {
        self.db.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                     FROM source_instances
                     WHERE id = ?1
                     LIMIT 1",
                    [id],
                    map_source_row,
                )
                .optional()
                .map_err(StorageError::from)
        })
    }

    pub fn list(&self) -> StorageResult<Vec<SourceInstance>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                 FROM source_instances
                 ORDER BY created_at, id",
            )?;
            let items = statement
                .query_map([], map_source_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn list_by_plugin(&self, plugin_id: &str) -> StorageResult<Vec<SourceInstance>> {
        self.db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, plugin_id, name, status, state_json, created_at, updated_at
                 FROM source_instances
                 WHERE plugin_id = ?1
                 ORDER BY created_at, id",
            )?;
            let items = statement
                .query_map([plugin_id], map_source_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
    }

    pub fn update(&self, source: &SourceInstance) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected = connection.execute(
                "UPDATE source_instances
                 SET plugin_id = ?1, name = ?2, status = ?3, state_json = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![
                    source.plugin_id,
                    source.name,
                    source.status,
                    source.state_json,
                    source.updated_at,
                    source.id
                ],
            )?;
            Ok(affected)
        })
    }

    pub fn delete(&self, id: &str) -> StorageResult<usize> {
        self.db.with_connection(|connection| {
            let affected =
                connection.execute("DELETE FROM source_instances WHERE id = ?1", [id])?;
            Ok(affected)
        })
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct NodeCacheEntry {
    pub id: String,
    pub source_instance_id: String,
    pub nodes: Vec<ProxyNode>,
    pub fetched_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshJob {
    pub id: String,
    pub source_instance_id: String,
    pub trigger_type: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub node_count: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportToken {
    pub id: String,
    pub profile_id: String,
    pub token: String,
    pub token_type: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

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

fn map_plugin_row(row: &Row<'_>) -> rusqlite::Result<Plugin> {
    Ok(Plugin {
        id: row.get("id")?,
        plugin_id: row.get("plugin_id")?,
        name: row.get("name")?,
        version: row.get("version")?,
        spec_version: row.get("spec_version")?,
        plugin_type: row.get("plugin_type")?,
        status: row.get("status")?,
        installed_at: row.get("installed_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_source_row(row: &Row<'_>) -> rusqlite::Result<SourceInstance> {
    Ok(SourceInstance {
        id: row.get("id")?,
        plugin_id: row.get("plugin_id")?,
        name: row.get("name")?,
        status: row.get("status")?,
        state_json: row.get("state_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_profile_row(row: &Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_setting_row(row: &Row<'_>) -> rusqlite::Result<AppSetting> {
    Ok(AppSetting {
        key: row.get("key")?,
        value: row.get("value")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_refresh_job_row(row: &Row<'_>) -> rusqlite::Result<RefreshJob> {
    Ok(RefreshJob {
        id: row.get("id")?,
        source_instance_id: row.get("source_instance_id")?,
        trigger_type: row.get("trigger_type")?,
        status: row.get("status")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        node_count: row.get("node_count")?,
        error_code: row.get("error_code")?,
        error_message: row.get("error_message")?,
    })
}

fn map_export_token_row(row: &Row<'_>) -> rusqlite::Result<ExportToken> {
    Ok(ExportToken {
        id: row.get("id")?,
        profile_id: row.get("profile_id")?,
        token: row.get("token")?,
        token_type: row.get("token_type")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use app_common::{
        AppSetting, Plugin, Profile, ProfileSource, ProxyNode, ProxyProtocol, ProxyTransport,
        SourceInstance, TlsConfig,
    };

    use super::Database;
    use super::ExportToken;
    use super::ExportTokenRepository;
    use super::NodeCacheRepository;
    use super::PluginRepository;
    use super::ProfileRepository;
    use super::RefreshJob;
    use super::RefreshJobRepository;
    use super::SettingsRepository;
    use super::SourceConfigRepository;
    use super::SourceRepository;
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

    #[test]
    fn plugin_repository_crud_workflow() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let repository = PluginRepository::new(&db);
        let plugin = sample_plugin("plugin-row-1", "vendor.example.static");

        repository.insert(&plugin)?;

        let by_id = repository.get_by_id(&plugin.id)?;
        assert_eq!(by_id, Some(plugin.clone()));

        let by_plugin_id = repository.get_by_plugin_id(&plugin.plugin_id)?;
        assert_eq!(by_plugin_id, Some(plugin.clone()));

        let list = repository.list()?;
        assert_eq!(list, vec![plugin.clone()]);

        let updated_at = "2026-04-02T02:00:00Z";
        assert_eq!(
            repository.update_status(&plugin.id, "disabled", updated_at)?,
            1
        );

        let updated = repository.get_by_id(&plugin.id)?.expect("插件应存在");
        assert_eq!(updated.status, "disabled");
        assert_eq!(updated.updated_at, updated_at);

        assert_eq!(repository.delete(&plugin.id)?, 1);
        assert!(repository.get_by_id(&plugin.id)?.is_none());

        Ok(())
    }

    #[test]
    fn source_repository_crud_workflow() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let repository = SourceRepository::new(&db);
        let source_a = sample_source("source-a", "vendor.example.static");
        let source_b = sample_source("source-b", "vendor.example.script");

        repository.insert(&source_a)?;
        repository.insert(&source_b)?;

        let by_id = repository.get_by_id(&source_a.id)?;
        assert_eq!(by_id, Some(source_a.clone()));

        let list = repository.list()?;
        assert_eq!(list.len(), 2);

        let list_by_plugin = repository.list_by_plugin(&source_a.plugin_id)?;
        assert_eq!(list_by_plugin, vec![source_a.clone()]);

        let mut updated_source = source_a.clone();
        updated_source.name = "Source A Updated".to_string();
        updated_source.status = "error".to_string();
        updated_source.state_json = Some("{\"last_error\":\"timeout\"}".to_string());
        updated_source.updated_at = "2026-04-02T02:30:00Z".to_string();
        assert_eq!(repository.update(&updated_source)?, 1);

        let loaded = repository
            .get_by_id(&updated_source.id)?
            .expect("来源应存在");
        assert_eq!(loaded, updated_source);

        assert_eq!(repository.delete(&updated_source.id)?, 1);
        assert!(repository.get_by_id(&updated_source.id)?.is_none());

        Ok(())
    }

    #[test]
    fn source_config_repository_replace_and_delete_workflow() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let source_repository = SourceRepository::new(&db);
        let config_repository = SourceConfigRepository::new(&db);
        let source = sample_source("source-config-1", "vendor.example.static");
        source_repository.insert(&source)?;

        let mut first = BTreeMap::new();
        first.insert("url".to_string(), "https://example.com/sub".to_string());
        first.insert("user_agent".to_string(), "SubForge/0.1".to_string());
        config_repository.replace_all(&source.id, &first)?;
        assert_eq!(config_repository.get_all(&source.id)?, first);

        let mut second = BTreeMap::new();
        second.insert("url".to_string(), "https://example.com/next".to_string());
        config_repository.replace_all(&source.id, &second)?;
        assert_eq!(config_repository.get_all(&source.id)?, second);

        assert_eq!(config_repository.delete_all(&source.id)?, 1);
        assert!(config_repository.get_all(&source.id)?.is_empty());

        Ok(())
    }

    #[test]
    fn profile_repository_crud_and_binding_workflow() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let profile_repository = ProfileRepository::new(&db);
        let source_repository = SourceRepository::new(&db);
        let profile = sample_profile("profile-default");
        let source = sample_source("source-for-profile", "vendor.example.static");

        source_repository.insert(&source)?;
        profile_repository.insert(&profile)?;

        let by_id = profile_repository.get_by_id(&profile.id)?;
        assert_eq!(by_id, Some(profile.clone()));

        let list = profile_repository.list()?;
        assert_eq!(list, vec![profile.clone()]);

        let mut updated_profile = profile.clone();
        updated_profile.name = "Profile Updated".to_string();
        updated_profile.description = Some("更新后的聚合配置".to_string());
        updated_profile.updated_at = "2026-04-02T03:00:00Z".to_string();
        assert_eq!(profile_repository.update(&updated_profile)?, 1);

        let loaded = profile_repository
            .get_by_id(&updated_profile.id)?
            .expect("配置应存在");
        assert_eq!(loaded, updated_profile);

        profile_repository.add_source(&updated_profile.id, &source.id, 10)?;
        profile_repository.add_source(&updated_profile.id, &source.id, 20)?;

        let profile_sources = list_profile_sources(&db, &updated_profile.id)?;
        assert_eq!(
            profile_sources,
            vec![ProfileSource {
                profile_id: updated_profile.id.clone(),
                source_instance_id: source.id.clone(),
                priority: 20,
            }]
        );

        assert_eq!(
            profile_repository.remove_source(&updated_profile.id, &source.id)?,
            1
        );
        let profile_sources = list_profile_sources(&db, &updated_profile.id)?;
        assert!(profile_sources.is_empty());

        assert_eq!(profile_repository.delete(&updated_profile.id)?, 1);
        assert!(profile_repository.get_by_id(&updated_profile.id)?.is_none());

        Ok(())
    }

    #[test]
    fn settings_repository_supports_upsert() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let repository = SettingsRepository::new(&db);

        assert!(repository.get("ui.theme")?.is_none());

        let setting = AppSetting {
            key: "ui.theme".to_string(),
            value: "dark".to_string(),
            updated_at: "2026-04-02T03:30:00Z".to_string(),
        };
        repository.set(&setting)?;
        assert_eq!(repository.get("ui.theme")?, Some(setting.clone()));

        let updated_setting = AppSetting {
            key: "ui.theme".to_string(),
            value: "light".to_string(),
            updated_at: "2026-04-02T03:31:00Z".to_string(),
        };
        repository.set(&updated_setting)?;
        assert_eq!(repository.get("ui.theme")?, Some(updated_setting.clone()));

        let secondary_setting = AppSetting {
            key: "core.port".to_string(),
            value: "18118".to_string(),
            updated_at: "2026-04-02T03:32:00Z".to_string(),
        };
        repository.set(&secondary_setting)?;

        let all = repository.get_all()?;
        assert_eq!(all, vec![secondary_setting, updated_setting]);

        Ok(())
    }

    #[test]
    fn node_cache_repository_upsert_and_delete_workflow() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let source_repository = SourceRepository::new(&db);
        let cache_repository = NodeCacheRepository::new(&db);
        let source = sample_source("source-cache-1", "vendor.example.static");
        source_repository.insert(&source)?;

        let first_nodes = vec![sample_proxy_node("node-a", "hk.example.com", 443)];
        cache_repository.upsert_nodes(
            &source.id,
            &first_nodes,
            "2026-04-02T04:00:00Z",
            Some("2026-04-02T05:00:00Z"),
        )?;

        let loaded = cache_repository
            .get_by_source(&source.id)?
            .expect("缓存应存在");
        assert_eq!(loaded.source_instance_id, source.id);
        assert_eq!(loaded.nodes, first_nodes);
        assert_eq!(loaded.fetched_at, "2026-04-02T04:00:00Z");
        assert_eq!(loaded.expires_at.as_deref(), Some("2026-04-02T05:00:00Z"));

        let second_nodes = vec![
            sample_proxy_node("node-b", "sg.example.com", 8443),
            sample_proxy_node("node-c", "us.example.com", 443),
        ];
        cache_repository.upsert_nodes(&source.id, &second_nodes, "2026-04-02T06:00:00Z", None)?;
        let updated = cache_repository
            .get_by_source(&source.id)?
            .expect("更新后缓存应存在");
        assert_eq!(updated.nodes, second_nodes);
        assert_eq!(updated.expires_at, None);

        assert_eq!(cache_repository.delete_by_source(&source.id)?, 1);
        assert!(cache_repository.get_by_source(&source.id)?.is_none());

        Ok(())
    }

    #[test]
    fn refresh_job_repository_records_success_and_failure() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let source_repository = SourceRepository::new(&db);
        let refresh_repository = RefreshJobRepository::new(&db);
        let source = sample_source("source-refresh-1", "vendor.example.static");
        source_repository.insert(&source)?;

        let success_job = RefreshJob {
            id: "refresh-job-success".to_string(),
            source_instance_id: source.id.clone(),
            trigger_type: "manual".to_string(),
            status: "running".to_string(),
            started_at: Some("2026-04-02T06:00:00Z".to_string()),
            finished_at: None,
            node_count: None,
            error_code: None,
            error_message: None,
        };
        refresh_repository.insert(&success_job)?;
        assert_eq!(
            refresh_repository.mark_success(&success_job.id, "2026-04-02T06:00:10Z", 42)?,
            1
        );

        let success_loaded = refresh_repository
            .get_by_id(&success_job.id)?
            .expect("成功任务应存在");
        assert_eq!(success_loaded.status, "success");
        assert_eq!(success_loaded.node_count, Some(42));
        assert_eq!(success_loaded.error_code, None);
        assert_eq!(success_loaded.error_message, None);

        let failed_job = RefreshJob {
            id: "refresh-job-failed".to_string(),
            source_instance_id: source.id.clone(),
            trigger_type: "scheduled".to_string(),
            status: "running".to_string(),
            started_at: Some("2026-04-02T06:10:00Z".to_string()),
            finished_at: None,
            node_count: None,
            error_code: None,
            error_message: None,
        };
        refresh_repository.insert(&failed_job)?;
        assert_eq!(
            refresh_repository.mark_failed(
                &failed_job.id,
                "2026-04-02T06:10:20Z",
                "E_HTTP_5XX",
                "upstream 502"
            )?,
            1
        );

        let failed_loaded = refresh_repository
            .get_by_id(&failed_job.id)?
            .expect("失败任务应存在");
        assert_eq!(failed_loaded.status, "failed");
        assert_eq!(failed_loaded.node_count, None);
        assert_eq!(failed_loaded.error_code.as_deref(), Some("E_HTTP_5XX"));
        assert_eq!(failed_loaded.error_message.as_deref(), Some("upstream 502"));

        let by_source = refresh_repository.list_by_source(&source.id)?;
        assert_eq!(by_source.len(), 2);
        assert_eq!(by_source[0].id, success_job.id);
        assert_eq!(by_source[1].id, failed_job.id);

        Ok(())
    }

    #[test]
    fn export_token_repository_supports_active_and_expiring_tokens() -> StorageResult<()> {
        let db = Database::open_in_memory()?;
        let profile_repository = ProfileRepository::new(&db);
        let token_repository = ExportTokenRepository::new(&db);
        let profile = sample_profile("profile-export-token");
        profile_repository.insert(&profile)?;

        let active = ExportToken {
            id: "token-active".to_string(),
            profile_id: profile.id.clone(),
            token: "token-active-value".to_string(),
            token_type: "primary".to_string(),
            created_at: "2026-04-02T06:20:00Z".to_string(),
            expires_at: None,
        };
        token_repository.insert(&active)?;

        let expiring = ExportToken {
            id: "token-expiring".to_string(),
            profile_id: profile.id.clone(),
            token: "token-expiring-value".to_string(),
            token_type: "grace".to_string(),
            created_at: "2026-04-02T06:21:00Z".to_string(),
            expires_at: Some("2026-04-02T06:30:00Z".to_string()),
        };
        token_repository.insert(&expiring)?;

        let loaded_active = token_repository
            .get_active_token(&profile.id)?
            .expect("应能读取 active token");
        assert_eq!(loaded_active.token, active.token);

        assert!(token_repository.is_valid_token(
            &profile.id,
            &active.token,
            "2026-04-02T06:22:00Z"
        )?);
        assert!(token_repository.is_valid_token(
            &profile.id,
            &expiring.token,
            "2026-04-02T06:22:00Z"
        )?);
        assert!(!token_repository.is_valid_token(
            &profile.id,
            &expiring.token,
            "2026-04-02T06:40:00Z"
        )?);

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

    fn list_profile_sources(db: &Database, profile_id: &str) -> StorageResult<Vec<ProfileSource>> {
        db.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT profile_id, source_instance_id, priority
                 FROM profile_sources
                 WHERE profile_id = ?1
                 ORDER BY source_instance_id",
            )?;

            let records = statement
                .query_map([profile_id], |row| {
                    Ok(ProfileSource {
                        profile_id: row.get("profile_id")?,
                        source_instance_id: row.get("source_instance_id")?,
                        priority: row.get("priority")?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(records)
        })
    }

    fn sample_plugin(id: &str, plugin_id: &str) -> Plugin {
        Plugin {
            id: id.to_string(),
            plugin_id: plugin_id.to_string(),
            name: "Example Plugin".to_string(),
            version: "1.0.0".to_string(),
            spec_version: "1.0".to_string(),
            plugin_type: "static".to_string(),
            status: "enabled".to_string(),
            installed_at: "2026-04-02T01:00:00Z".to_string(),
            updated_at: "2026-04-02T01:00:00Z".to_string(),
        }
    }

    fn sample_source(id: &str, plugin_id: &str) -> SourceInstance {
        SourceInstance {
            id: id.to_string(),
            plugin_id: plugin_id.to_string(),
            name: format!("Source {id}"),
            status: "healthy".to_string(),
            state_json: None,
            created_at: "2026-04-02T01:10:00Z".to_string(),
            updated_at: "2026-04-02T01:10:00Z".to_string(),
        }
    }

    fn sample_proxy_node(id: &str, server: &str, port: u16) -> ProxyNode {
        ProxyNode {
            id: id.to_string(),
            name: format!("{server}:{port}"),
            protocol: ProxyProtocol::Ss,
            server: server.to_string(),
            port,
            transport: ProxyTransport::Tcp,
            tls: TlsConfig {
                enabled: true,
                server_name: Some(server.to_string()),
            },
            extra: BTreeMap::new(),
            source_id: "source-cache-1".to_string(),
            tags: Vec::new(),
            region: None,
            updated_at: "2026-04-02T04:00:00Z".to_string(),
        }
    }

    fn sample_profile(id: &str) -> Profile {
        Profile {
            id: id.to_string(),
            name: "Default Profile".to_string(),
            description: Some("默认配置".to_string()),
            created_at: "2026-04-02T01:20:00Z".to_string(),
            updated_at: "2026-04-02T01:20:00Z".to_string(),
        }
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
