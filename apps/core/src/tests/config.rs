use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_secrets::{MemorySecretStore, SecretStore};
use app_storage::{Database, ProfileRepository};
use serde_json::json;

use crate::config::LoadedHeadlessConfig;
use crate::headless::{apply_headless_configuration, list_profile_source_ids, list_sources};

#[test]
fn headless_config_resolves_env_secret_and_listen_addr() {
    let temp_root = create_temp_dir("headless-config-parse");
    let config_path = temp_root.join("subforge.toml");
    let plugin_dir = builtin_plugin_dir();
    let env_key = "SUBFORGE_TEST_HEADLESS_PASSWORD";

    unsafe {
        std::env::set_var(env_key, "env-password");
    }

    std::fs::write(
        &config_path,
        format!(
            r#"
[server]
listen = "127.0.0.1:19118"

[plugins]
dirs = ["{plugin_dir}"]

[[sources]]
name = "static-a"
plugin = "subforge.builtin.static"
[sources.config]
url = "https://example.com/sub"
[sources.secrets]
password = {{ env = "{env_key}" }}

[[profiles]]
name = "main"
sources = ["static-a"]
"#,
            plugin_dir = path_to_toml_string(&plugin_dir),
            env_key = env_key
        ),
    )
    .expect("写入配置文件失败");

    let loaded = LoadedHeadlessConfig::from_file(&config_path).expect("加载配置失败");
    let (host, port) = loaded.listen_host_port().expect("解析监听地址失败");
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 19118);

    let resolved = loaded
        .resolve_source_config(&loaded.config.sources[0])
        .expect("解析来源配置失败");
    assert_eq!(
        resolved.get("password"),
        Some(&json!("env-password")),
        "应将 env secret 解析为来源配置字段"
    );

    unsafe {
        std::env::remove_var(env_key);
    }
}

#[test]
fn apply_headless_configuration_is_idempotent_for_sources_and_profiles() {
    let temp_root = create_temp_dir("headless-apply");
    let config_path = temp_root.join("subforge.toml");
    let plugin_dir = builtin_plugin_dir();

    std::fs::write(
        &config_path,
        format!(
            r#"
[plugins]
dirs = ["{plugin_dir}"]

[[sources]]
name = "static-a"
plugin = "subforge.builtin.static"
[sources.config]
url = "https://example.com/sub"

[[profiles]]
name = "main"
sources = ["static-a"]
"#,
            plugin_dir = path_to_toml_string(&plugin_dir)
        ),
    )
    .expect("写入配置文件失败");

    let loaded = LoadedHeadlessConfig::from_file(&config_path).expect("加载配置失败");
    let database = Database::open_in_memory().expect("初始化内存数据库失败");
    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let runtime_plugins_dir = temp_root.join("runtime-plugins");
    std::fs::create_dir_all(&runtime_plugins_dir).expect("创建 runtime 插件目录失败");

    let first = apply_headless_configuration(
        &loaded,
        &database,
        Arc::clone(&secret_store),
        &runtime_plugins_dir,
    )
    .expect("首次应用无头配置失败");
    assert_eq!(first.created_sources, 1);
    assert_eq!(first.created_profiles, 1);

    let second = apply_headless_configuration(
        &loaded,
        &database,
        Arc::clone(&secret_store),
        &runtime_plugins_dir,
    )
    .expect("重复应用无头配置失败");
    assert_eq!(second.created_sources, 0);
    assert_eq!(second.created_profiles, 0);
    assert_eq!(second.updated_sources, 1);
    assert_eq!(second.updated_profiles, 1);

    let sources = list_sources(&database).expect("读取来源列表失败");
    let profiles = ProfileRepository::new(&database)
        .list()
        .expect("读取 Profile 列表失败");
    assert_eq!(sources.len(), 1, "重复应用后来源数量不应增长");
    assert_eq!(profiles.len(), 1, "重复应用后 Profile 数量不应增长");

    let profile_sources =
        list_profile_source_ids(&database, &profiles[0].id).expect("读取 Profile 关联来源失败");
    assert_eq!(profile_sources, vec![sources[0].id.clone()]);
}

fn builtin_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/builtins/static")
}

fn path_to_toml_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "subforge-{prefix}-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}
