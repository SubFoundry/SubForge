use anyhow::{Context, Result};
use app_common::AppSetting;
use app_storage::{Database, SettingsRepository};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cli::RunArgs;

pub(crate) fn seed_default_settings(database: &Database, args: &RunArgs) -> Result<()> {
    let repository = SettingsRepository::new(database);
    let updated_at = current_timestamp_rfc3339()?;

    let defaults = [
        ("http_listen_addr", args.host.clone()),
        ("http_listen_port", args.port.to_string()),
        ("log_level", "info".to_string()),
        ("log_retention_days", "7".to_string()),
        ("theme", "dark".to_string()),
        ("auto_refresh_on_start", "true".to_string()),
        ("tray_minimize", "true".to_string()),
        ("gui_idle_auto_close_minutes", "30".to_string()),
        ("gui_close_behavior", "tray_minimize".to_string()),
    ];

    for (key, value) in defaults {
        set_default_setting_if_absent(&repository, key, value, &updated_at)?;
    }

    Ok(())
}

pub(crate) fn set_default_setting_if_absent(
    repository: &SettingsRepository<'_>,
    key: &str,
    value: String,
    updated_at: &str,
) -> Result<()> {
    if repository.get(key)?.is_none() {
        repository.set(&AppSetting {
            key: key.to_string(),
            value,
            updated_at: updated_at.to_string(),
        })?;
    }
    Ok(())
}

fn current_timestamp_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("格式化 RFC3339 时间戳失败")
}
