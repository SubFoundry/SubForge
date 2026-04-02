use app_common::{AppSetting, Plugin, Profile, SourceInstance};
use rusqlite::Row;

use crate::{ExportToken, RefreshJob};
pub(crate) fn map_plugin_row(row: &Row<'_>) -> rusqlite::Result<Plugin> {
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

pub(crate) fn map_source_row(row: &Row<'_>) -> rusqlite::Result<SourceInstance> {
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

pub(crate) fn map_profile_row(row: &Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(crate) fn map_setting_row(row: &Row<'_>) -> rusqlite::Result<AppSetting> {
    Ok(AppSetting {
        key: row.get("key")?,
        value: row.get("value")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(crate) fn map_refresh_job_row(row: &Row<'_>) -> rusqlite::Result<RefreshJob> {
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

pub(crate) fn map_export_token_row(row: &Row<'_>) -> rusqlite::Result<ExportToken> {
    Ok(ExportToken {
        id: row.get("id")?,
        profile_id: row.get("profile_id")?,
        token: row.get("token")?,
        token_type: row.get("token_type")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}
