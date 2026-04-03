use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use app_common::{AppSetting, Profile};
use app_core::{CoreError, PluginInstallService, SourceService};
use app_secrets::SecretStore;
use app_storage::{Database, ProfileRepository};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::LoadedHeadlessConfig;
use crate::headless::HeadlessApplyReport;
use crate::headless::discovery::discover_plugins_from_dirs;
use crate::headless::token::ensure_export_token;

pub(crate) fn apply_headless_configuration(
    loaded: &LoadedHeadlessConfig,
    database: &Database,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: &Path,
) -> Result<HeadlessApplyReport> {
    let mut report = HeadlessApplyReport::default();
    let discovered_plugins = discover_plugins_from_dirs(&loaded.resolved_plugins_dirs()?)?;
    let install_service = PluginInstallService::new(database, plugins_dir.to_path_buf());
    for plugin_root in discovered_plugins {
        match install_service.install_from_dir(&plugin_root) {
            Ok(_) => {
                report.installed_plugins += 1;
            }
            Err(CoreError::PluginAlreadyInstalled(_)) => {}
            Err(error) => return Err(anyhow!("安装插件失败: {error}")),
        }
    }

    let source_name_to_id = apply_sources(
        loaded,
        database,
        Arc::clone(&secret_store),
        plugins_dir,
        &mut report,
    )?;
    apply_profiles(
        loaded,
        database,
        Arc::clone(&secret_store),
        plugins_dir,
        &source_name_to_id,
        &mut report,
    )?;
    Ok(report)
}

pub(crate) fn apply_headless_settings(
    loaded: &LoadedHeadlessConfig,
    database: &Database,
) -> Result<()> {
    let repository = app_storage::SettingsRepository::new(database);
    let now = now_rfc3339()?;
    let mut settings = Vec::new();

    let (host, port) = loaded.listen_host_port()?;
    settings.push(AppSetting {
        key: "http_listen_addr".to_string(),
        value: host,
        updated_at: now.clone(),
    });
    settings.push(AppSetting {
        key: "http_listen_port".to_string(),
        value: port.to_string(),
        updated_at: now.clone(),
    });
    settings.push(AppSetting {
        key: "log_level".to_string(),
        value: loaded.config.log.level.clone(),
        updated_at: now.clone(),
    });
    settings.push(AppSetting {
        key: "log_retention_days".to_string(),
        value: loaded.config.log.retention_days.to_string(),
        updated_at: now.clone(),
    });
    settings.push(AppSetting {
        key: "auto_refresh_on_start".to_string(),
        value: loaded.config.refresh.auto_on_start.to_string(),
        updated_at: now.clone(),
    });
    settings.push(AppSetting {
        key: "refresh_default_interval_sec".to_string(),
        value: loaded.config.refresh.default_interval_sec.to_string(),
        updated_at: now.clone(),
    });
    if let Some(log_dir) = loaded.resolved_log_dir()? {
        settings.push(AppSetting {
            key: "log_dir".to_string(),
            value: log_dir.to_string_lossy().into_owned(),
            updated_at: now,
        });
    }

    for setting in settings {
        repository.set(&setting)?;
    }
    Ok(())
}

fn apply_sources(
    loaded: &LoadedHeadlessConfig,
    database: &Database,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: &Path,
    report: &mut HeadlessApplyReport,
) -> Result<BTreeMap<String, String>> {
    let service = SourceService::new(database, plugins_dir.to_path_buf(), secret_store.as_ref());
    let existing = service
        .list_sources()
        .map_err(|error| anyhow!("读取现有来源失败: {error}"))?;
    let mut existing_by_name = BTreeMap::new();
    for source in existing {
        if existing_by_name
            .insert(source.source.name.clone(), source.source)
            .is_some()
        {
            bail!("数据库中存在重名来源，无法执行无头配置同步");
        }
    }

    let mut source_name_to_id = BTreeMap::new();
    for source in &loaded.config.sources {
        let effective_config = loaded.resolve_source_config(source)?;
        if let Some(existing_source) = existing_by_name.get(&source.name) {
            if existing_source.plugin_id != source.plugin {
                bail!(
                    "来源 {} 的 plugin 与已存在记录不一致（{} != {}）",
                    source.name,
                    existing_source.plugin_id,
                    source.plugin
                );
            }
            service
                .update_source_config(&existing_source.id, effective_config)
                .map_err(|error| anyhow!("更新来源失败（{}）: {error}", source.name))?;
            source_name_to_id.insert(source.name.clone(), existing_source.id.clone());
            report.updated_sources += 1;
            continue;
        }

        let created = service
            .create_source(&source.plugin, &source.name, effective_config)
            .map_err(|error| anyhow!("创建来源失败（{}）: {error}", source.name))?;
        source_name_to_id.insert(source.name.clone(), created.source.id.clone());
        report.created_sources += 1;
    }
    Ok(source_name_to_id)
}

fn apply_profiles(
    loaded: &LoadedHeadlessConfig,
    database: &Database,
    secret_store: Arc<dyn SecretStore>,
    plugins_dir: &Path,
    source_name_to_id: &BTreeMap<String, String>,
    report: &mut HeadlessApplyReport,
) -> Result<()> {
    let repository = ProfileRepository::new(database);
    let existing_profiles = repository
        .list()
        .map_err(|error| anyhow!("读取现有 Profile 失败: {error}"))?;
    let mut existing_by_name = BTreeMap::new();
    for profile in existing_profiles {
        if existing_by_name
            .insert(profile.name.clone(), profile)
            .is_some()
        {
            bail!("数据库中存在重名 Profile，无法执行无头配置同步");
        }
    }

    for profile in &loaded.config.profiles {
        let source_ids = profile
            .sources
            .iter()
            .map(|name| {
                source_name_to_id
                    .get(name)
                    .cloned()
                    .ok_or_else(|| anyhow!("Profile {} 引用了未知来源：{}", profile.name, name))
            })
            .collect::<Result<Vec<_>>>()?;

        let profile_id = if let Some(existing) = existing_by_name.get(&profile.name) {
            let mut next = existing.clone();
            next.description = profile.description.clone();
            next.updated_at = now_rfc3339()?;
            repository
                .update(&next)
                .map_err(|error| anyhow!("更新 Profile 失败（{}）: {error}", profile.name))?;
            replace_profile_sources(database, &next.id, &source_ids)?;
            report.updated_profiles += 1;
            next.id
        } else {
            let now = now_rfc3339()?;
            let created = Profile {
                id: format!(
                    "profile-{}",
                    OffsetDateTime::now_utc().unix_timestamp_nanos()
                ),
                name: profile.name.clone(),
                description: profile.description.clone(),
                created_at: now.clone(),
                updated_at: now,
            };
            repository
                .insert(&created)
                .map_err(|error| anyhow!("创建 Profile 失败（{}）: {error}", profile.name))?;
            replace_profile_sources(database, &created.id, &source_ids)?;
            report.created_profiles += 1;
            created.id
        };

        ensure_export_token(
            database,
            plugins_dir,
            Arc::clone(&secret_store),
            profile,
            &profile_id,
        )?;
    }
    Ok(())
}

fn replace_profile_sources(
    database: &Database,
    profile_id: &str,
    source_ids: &[String],
) -> Result<()> {
    database
        .with_connection(|connection| {
            let tx = connection.transaction()?;
            tx.execute(
                "DELETE FROM profile_sources WHERE profile_id = ?1",
                [profile_id],
            )?;
            for (index, source_id) in source_ids.iter().enumerate() {
                tx.execute(
                    "INSERT INTO profile_sources (profile_id, source_instance_id, priority)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![profile_id, source_id, index as i64],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .map_err(Into::into)
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("格式化时间失败")
}
