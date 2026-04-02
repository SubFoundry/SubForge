//! app-storage：存储层（SQLite、迁移、仓储接口）。

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use app_common::{AppSetting, Plugin, Profile, SourceInstance};
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use app_common::{AppSetting, Plugin, Profile, ProfileSource, SourceInstance};

    use super::Database;
    use super::PluginRepository;
    use super::ProfileRepository;
    use super::SettingsRepository;
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
