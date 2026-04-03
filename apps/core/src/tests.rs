use app_common::AppSetting;
use app_storage::{Database, SettingsRepository, StorageResult};

use crate::bootstrap::run_refresh;
use crate::cli::RefreshArgs;
use crate::settings_seed::set_default_setting_if_absent;

mod config;
mod headless_runtime;

#[test]
fn set_default_setting_only_writes_when_missing() -> StorageResult<()> {
    let db = Database::open_in_memory()?;
    let repository = SettingsRepository::new(&db);

    set_default_setting_if_absent(
        &repository,
        "ui.theme",
        "dark".to_string(),
        "2026-04-02T00:00:00Z",
    )
    .expect("首次设置默认值失败");
    set_default_setting_if_absent(
        &repository,
        "ui.theme",
        "light".to_string(),
        "2026-04-02T00:10:00Z",
    )
    .expect("重复设置默认值失败");

    let loaded = repository.get("ui.theme")?.expect("应存在默认配置");
    assert_eq!(
        loaded,
        AppSetting {
            key: "ui.theme".to_string(),
            value: "dark".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string()
        }
    );

    Ok(())
}

#[test]
fn refresh_placeholder_keeps_behavior() {
    run_refresh(RefreshArgs { source_id: None }).expect("全量刷新占位应返回成功");
    run_refresh(RefreshArgs {
        source_id: Some("source-1".to_string()),
    })
    .expect("单来源刷新占位应返回成功");
}
