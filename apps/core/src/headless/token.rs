use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use app_core::Engine;
use app_secrets::SecretStore;
use app_storage::{Database, ExportToken, ExportTokenRepository};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::ProfileSection;

pub(super) fn ensure_export_token(
    database: &Database,
    plugins_dir: &Path,
    secret_store: Arc<dyn SecretStore>,
    profile: &ProfileSection,
    profile_id: &str,
) -> Result<()> {
    let repository = ExportTokenRepository::new(database);
    if let Some(configured_token) = profile.export_token.as_ref() {
        let now = now_rfc3339()?;
        if let Some(active) = repository.get_active_token(profile_id)? {
            if active.token == *configured_token {
                return Ok(());
            }
            let replacement = ExportToken {
                id: format!(
                    "export-token-{}",
                    OffsetDateTime::now_utc().unix_timestamp_nanos()
                ),
                profile_id: profile_id.to_string(),
                token: configured_token.clone(),
                token_type: "primary".to_string(),
                created_at: now.clone(),
                expires_at: None,
            };
            repository.rotate_primary_token_with_grace(profile_id, &replacement, &now, &now)?;
            return Ok(());
        }
        let token = ExportToken {
            id: format!(
                "export-token-{}",
                OffsetDateTime::now_utc().unix_timestamp_nanos()
            ),
            profile_id: profile_id.to_string(),
            token: configured_token.clone(),
            token_type: "primary".to_string(),
            created_at: now,
            expires_at: None,
        };
        repository.insert(&token)?;
        return Ok(());
    }

    let engine = Engine::new(database, plugins_dir, secret_store);
    engine
        .ensure_profile_export_token(profile_id)
        .map_err(|error| anyhow!("确保 export token 失败（{profile_id}）: {error}"))?;
    Ok(())
}

fn now_rfc3339() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
