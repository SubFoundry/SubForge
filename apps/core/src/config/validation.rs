use std::collections::BTreeSet;

use anyhow::{Result, bail};

use crate::config::LoadedHeadlessConfig;

pub(super) fn validate_loaded_config(loaded: &LoadedHeadlessConfig) -> Result<()> {
    loaded.parse_listen_addr()?;
    validate_log_level(&loaded.config.log.level)?;
    validate_refresh_interval(
        loaded.config.refresh.default_interval_sec,
        "refresh.default_interval_sec",
    )?;

    if let Some(token) = &loaded.config.server.admin_token {
        if token.trim().is_empty() {
            bail!("server.admin_token 不能为空字符串");
        }
    }

    let mut source_names = BTreeSet::new();
    for source in &loaded.config.sources {
        if source.name.trim().is_empty() {
            bail!("sources.name 不能为空");
        }
        if source.plugin.trim().is_empty() {
            bail!("sources.plugin 不能为空（source: {}）", source.name);
        }
        if !source_names.insert(source.name.clone()) {
            bail!("sources.name 重复: {}", source.name);
        }
        if let Some(interval) = source.refresh_interval_sec {
            validate_refresh_interval(interval, "sources.refresh_interval_sec")?;
        }
        if let Some(profile) = &source.network_profile {
            validate_network_profile(profile)?;
        }
        for (secret_key, secret_value) in &source.secrets {
            match (&secret_value.env, &secret_value.value) {
                (Some(env_name), None) => {
                    if env_name.trim().is_empty() {
                        bail!("sources.secrets.{secret_key}.env 不能为空");
                    }
                }
                (None, Some(_)) => {}
                (Some(_), Some(_)) => {
                    bail!("sources.secrets.{secret_key} 同时配置 env 和 value");
                }
                (None, None) => {
                    bail!("sources.secrets.{secret_key} 必须配置 env 或 value");
                }
            }
        }
    }

    let source_set = loaded
        .config
        .sources
        .iter()
        .map(|source| source.name.clone())
        .collect::<BTreeSet<_>>();
    let mut profile_names = BTreeSet::new();
    for profile in &loaded.config.profiles {
        if profile.name.trim().is_empty() {
            bail!("profiles.name 不能为空");
        }
        if !profile_names.insert(profile.name.clone()) {
            bail!("profiles.name 重复: {}", profile.name);
        }
        for source_name in &profile.sources {
            if !source_set.contains(source_name) {
                bail!(
                    "profiles.sources 引用了未定义来源: {source_name}（profile: {}）",
                    profile.name
                );
            }
        }
        if let Some(token) = &profile.export_token {
            if token.trim().is_empty() {
                bail!(
                    "profiles.export_token 不能为空字符串（profile: {}）",
                    profile.name
                );
            }
        }
    }

    for dir in loaded.resolved_plugins_dirs()? {
        if !dir.exists() {
            bail!("plugins.dirs 路径不存在: {}", dir.display());
        }
    }

    Ok(())
}

fn validate_log_level(level: &str) -> Result<()> {
    let normalized = level.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "trace" | "debug" | "info" | "warn" | "error"
    ) {
        Ok(())
    } else {
        bail!("log.level 仅支持 trace/debug/info/warn/error，实际: {level}");
    }
}

fn validate_network_profile(profile: &str) -> Result<()> {
    if matches!(
        profile,
        "standard" | "browser_chrome" | "browser_firefox" | "webview_assisted"
    ) {
        Ok(())
    } else {
        bail!(
            "sources.network_profile 仅支持 standard/browser_chrome/browser_firefox/webview_assisted，实际: {profile}"
        );
    }
}

fn validate_refresh_interval(value: u64, field: &str) -> Result<()> {
    if (120..=86_400).contains(&value) {
        Ok(())
    } else {
        bail!("{field} 必须在 120..=86400 之间，实际: {value}");
    }
}
