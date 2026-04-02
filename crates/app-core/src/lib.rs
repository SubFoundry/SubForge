//! app-core：业务编排层（调度、刷新、重试、状态机）。

use std::fs;
use std::path::{Path, PathBuf};

use app_common::Plugin;
use app_plugin_runtime::{PluginLoader, PluginRuntimeError};
use app_storage::{Database, PluginRepository, StorageError};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("插件运行时错误：{0}")]
    PluginRuntime(#[from] PluginRuntimeError),
    #[error("存储层错误：{0}")]
    Storage(#[from] StorageError),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("时间格式化失败：{0}")]
    TimeFormat(#[from] time::error::Format),
    #[error("插件已安装：{0}")]
    PluginAlreadyInstalled(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug)]
pub struct PluginInstallService<'a> {
    db: &'a Database,
    loader: PluginLoader,
    plugins_dir: PathBuf,
}

impl<'a> PluginInstallService<'a> {
    pub fn new(db: &'a Database, plugins_dir: impl Into<PathBuf>) -> Self {
        Self {
            db,
            loader: PluginLoader::new(),
            plugins_dir: plugins_dir.into(),
        }
    }

    pub fn install_from_dir(&self, source_dir: impl AsRef<Path>) -> CoreResult<Plugin> {
        let loaded = self.loader.load_from_dir(source_dir)?;
        let repository = PluginRepository::new(self.db);
        let existing_plugin = repository.get_by_plugin_id(&loaded.manifest.plugin_id)?;

        fs::create_dir_all(&self.plugins_dir)?;
        let target_dir = self.plugins_dir.join(&loaded.manifest.plugin_id);
        if let Some(existing) = existing_plugin {
            if existing.version == loaded.manifest.version {
                return Err(CoreError::PluginAlreadyInstalled(
                    loaded.manifest.plugin_id.clone(),
                ));
            }

            if target_dir.exists() {
                fs::remove_dir_all(&target_dir)?;
            }
            repository.delete(&existing.id)?;
        }

        if target_dir.exists() {
            return Err(CoreError::PluginAlreadyInstalled(
                loaded.manifest.plugin_id.clone(),
            ));
        }
        copy_dir_recursive(&loaded.root_dir, &target_dir)?;

        let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let plugin = Plugin {
            id: format!(
                "{}-{}",
                loaded.manifest.plugin_id,
                OffsetDateTime::now_utc().unix_timestamp_nanos()
            ),
            plugin_id: loaded.manifest.plugin_id,
            name: loaded.manifest.name,
            version: loaded.manifest.version,
            spec_version: loaded.manifest.spec_version,
            plugin_type: loaded.manifest.plugin_type.as_str().to_string(),
            status: "installed".to_string(),
            installed_at: now.clone(),
            updated_at: now,
        };

        if let Err(error) = repository.insert(&plugin) {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(error.into());
        }

        Ok(plugin)
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> CoreResult<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use app_storage::{Database, PluginRepository};

    use super::{CoreError, PluginInstallService};

    #[test]
    fn install_plugin_copies_files_and_inserts_database_record() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-success");
        let plugins_dir = temp_root.join("plugins");
        let service = PluginInstallService::new(&db, &plugins_dir);

        let source = builtins_static_plugin_dir();
        let installed = service
            .install_from_dir(&source)
            .expect("安装内置插件应成功");

        let target_dir = plugins_dir.join("subforge.builtin.static");
        assert!(target_dir.join("plugin.json").is_file());
        assert!(target_dir.join("schema.json").is_file());
        assert_eq!(installed.plugin_id, "subforge.builtin.static");
        assert_eq!(installed.status, "installed");

        let repository = PluginRepository::new(&db);
        let loaded = repository
            .get_by_plugin_id("subforge.builtin.static")
            .expect("查询已安装插件失败")
            .expect("数据库中应存在插件记录");
        assert_eq!(loaded.plugin_id, "subforge.builtin.static");

        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_same_plugin_twice_returns_error() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-duplicate");
        let plugins_dir = temp_root.join("plugins");
        let service = PluginInstallService::new(&db, &plugins_dir);
        let source = builtins_static_plugin_dir();

        service.install_from_dir(&source).expect("首次安装应成功");
        let duplicate_error = service
            .install_from_dir(&source)
            .expect_err("重复安装应失败");

        assert!(matches!(
            duplicate_error,
            CoreError::PluginAlreadyInstalled(_)
        ));
        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_higher_version_plugin_treats_as_upgrade() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-upgrade");
        let plugins_dir = temp_root.join("plugins");
        let upgraded_source = create_upgraded_plugin_dir(&temp_root);
        let service = PluginInstallService::new(&db, &plugins_dir);
        let source = builtins_static_plugin_dir();

        let installed_v1 = service.install_from_dir(&source).expect("首次安装应成功");
        assert_eq!(installed_v1.version, "1.0.0");

        let installed_v2 = service
            .install_from_dir(&upgraded_source)
            .expect("升级安装应成功");
        assert_eq!(installed_v2.version, "1.0.1");

        let repository = PluginRepository::new(&db);
        let loaded = repository
            .get_by_plugin_id("subforge.builtin.static")
            .expect("查询升级后插件失败")
            .expect("升级后插件记录应存在");
        assert_eq!(loaded.version, "1.0.1");

        cleanup_dir(&temp_root);
    }

    #[test]
    fn install_invalid_plugin_keeps_target_directory_clean() {
        let db = Database::open_in_memory().expect("内存数据库初始化失败");
        let temp_root = create_temp_dir("install-invalid");
        let plugins_dir = temp_root.join("plugins");
        let bad_plugin_dir = create_bad_plugin_dir(&temp_root);
        let service = PluginInstallService::new(&db, &plugins_dir);

        let error = service
            .install_from_dir(&bad_plugin_dir)
            .expect_err("非法插件安装应失败");
        assert!(matches!(error, CoreError::PluginRuntime(_)));

        let entries = fs::read_dir(&plugins_dir)
            .ok()
            .into_iter()
            .flat_map(|iter| iter.filter_map(Result::ok))
            .collect::<Vec<_>>();
        assert!(entries.is_empty(), "非法插件不应留下安装目录");

        cleanup_dir(&temp_root);
    }

    fn builtins_static_plugin_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
    }

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间异常")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("subforge-app-core-{prefix}-{nanos}"));
        fs::create_dir_all(&path).expect("创建临时目录失败");
        path
    }

    fn create_bad_plugin_dir(base: &Path) -> PathBuf {
        let path = base.join("invalid-plugin");
        fs::create_dir_all(&path).expect("创建非法插件目录失败");
        fs::write(
            path.join("plugin.json"),
            r#"{
                "plugin_id": "vendor.example.invalid",
                "spec_version": "1.0",
                "name": "Invalid Plugin",
                "version": "1.0.0",
                "type": "static",
                "config_schema": "schema.json"
            }"#,
        )
        .expect("写入非法插件 plugin.json 失败");
        fs::write(path.join("schema.json"), r#"{"type":"object","oneOf":[]}"#)
            .expect("写入非法插件 schema.json 失败");
        path
    }

    fn create_upgraded_plugin_dir(base: &Path) -> PathBuf {
        let path = base.join("upgraded-plugin");
        fs::create_dir_all(&path).expect("创建升级插件目录失败");
        fs::copy(
            builtins_static_plugin_dir().join("schema.json"),
            path.join("schema.json"),
        )
        .expect("复制 schema.json 失败");
        let plugin_json = fs::read_to_string(builtins_static_plugin_dir().join("plugin.json"))
            .expect("读取内置 plugin.json 失败")
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.0.1\"");
        fs::write(path.join("plugin.json"), plugin_json).expect("写入升级插件 plugin.json 失败");
        path
    }

    fn cleanup_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
