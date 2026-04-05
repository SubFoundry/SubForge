use crate::{
    Database, ExportToken, ExportTokenRepository, NodeCacheRepository, ProfileRepository,
    RefreshJob, RefreshJobRepository, ScriptLog, ScriptLogRepository, SourceRepository,
    StorageResult,
};

use super::support::{sample_profile, sample_proxy_node, sample_source};

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

    let recent = refresh_repository.list_recent(10)?;
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, failed_job.id);
    assert_eq!(recent[1].id, success_job.id);

    let failed_recent = refresh_repository.list_recent_by_status("failed", 10)?;
    assert_eq!(failed_recent.len(), 1);
    assert_eq!(failed_recent[0].id, failed_job.id);

    Ok(())
}

#[test]
fn refresh_job_repository_can_finalize_running_jobs_by_source() -> StorageResult<()> {
    let db = Database::open_in_memory()?;
    let source_repository = SourceRepository::new(&db);
    let refresh_repository = RefreshJobRepository::new(&db);
    let source = sample_source("source-refresh-running-cleanup", "vendor.example.script");
    source_repository.insert(&source)?;

    refresh_repository.insert(&RefreshJob {
        id: "refresh-job-running-1".to_string(),
        source_instance_id: source.id.clone(),
        trigger_type: "manual".to_string(),
        status: "running".to_string(),
        started_at: Some("2026-04-02T11:00:00Z".to_string()),
        finished_at: None,
        node_count: None,
        error_code: None,
        error_message: None,
    })?;
    refresh_repository.insert(&RefreshJob {
        id: "refresh-job-running-2".to_string(),
        source_instance_id: source.id.clone(),
        trigger_type: "scheduled".to_string(),
        status: "running".to_string(),
        started_at: Some("2026-04-02T11:01:00Z".to_string()),
        finished_at: None,
        node_count: None,
        error_code: None,
        error_message: None,
    })?;
    refresh_repository.insert(&RefreshJob {
        id: "refresh-job-success-existing".to_string(),
        source_instance_id: source.id.clone(),
        trigger_type: "scheduled".to_string(),
        status: "success".to_string(),
        started_at: Some("2026-04-02T11:02:00Z".to_string()),
        finished_at: Some("2026-04-02T11:02:10Z".to_string()),
        node_count: Some(3),
        error_code: None,
        error_message: None,
    })?;

    assert_eq!(
        refresh_repository.mark_running_failed_by_source(
            &source.id,
            "2026-04-02T11:03:00Z",
            "E_INTERNAL",
            "source refresh superseded",
        )?,
        2
    );

    let jobs = refresh_repository.list_by_source(&source.id)?;
    let running = jobs
        .iter()
        .filter(|job| job.status == "running")
        .collect::<Vec<_>>();
    assert!(running.is_empty());

    let failed = jobs
        .iter()
        .filter(|job| job.status == "failed")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|job| {
        job.error_code.as_deref() == Some("E_INTERNAL")
            && job.error_message.as_deref() == Some("source refresh superseded")
    }));

    let success = jobs
        .iter()
        .find(|job| job.id == "refresh-job-success-existing")
        .expect("existing success job should remain");
    assert_eq!(success.status, "success");

    Ok(())
}

#[test]
fn refresh_job_repository_supports_filtered_pagination() -> StorageResult<()> {
    let db = Database::open_in_memory()?;
    let source_repository = SourceRepository::new(&db);
    let refresh_repository = RefreshJobRepository::new(&db);
    let source_a = sample_source("source-refresh-a", "vendor.example.static");
    let source_b = sample_source("source-refresh-b", "vendor.example.static");
    source_repository.insert(&source_a)?;
    source_repository.insert(&source_b)?;

    for index in 0..5 {
        let source_id = if index % 2 == 0 {
            source_a.id.as_str()
        } else {
            source_b.id.as_str()
        };
        let status = if index % 3 == 0 { "failed" } else { "success" };
        let started = format!("2026-04-02T06:1{index}:00Z");
        let finished = format!("2026-04-02T06:1{index}:30Z");

        let job = RefreshJob {
            id: format!("refresh-filter-{index}"),
            source_instance_id: source_id.to_string(),
            trigger_type: "manual".to_string(),
            status: status.to_string(),
            started_at: Some(started),
            finished_at: Some(finished),
            node_count: Some((10 + index) as i64),
            error_code: (status == "failed").then(|| "E_HTTP_5XX".to_string()),
            error_message: (status == "failed").then(|| "upstream 502".to_string()),
        };
        refresh_repository.insert(&job)?;
    }

    let page = refresh_repository.list_recent_filtered(None, None, 2, 1)?;
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, "refresh-filter-3");
    assert_eq!(page[1].id, "refresh-filter-2");

    let failed_for_source_a =
        refresh_repository.list_recent_filtered(Some("failed"), Some(&source_a.id), 10, 0)?;
    assert_eq!(failed_for_source_a.len(), 1);
    assert_eq!(failed_for_source_a[0].source_instance_id, source_a.id);
    assert_eq!(failed_for_source_a[0].status, "failed");

    assert_eq!(refresh_repository.count_filtered(None, None)?, 5);
    assert_eq!(refresh_repository.count_filtered(Some("failed"), None)?, 2);
    assert_eq!(
        refresh_repository.count_filtered(Some("success"), Some(&source_b.id))?,
        1
    );

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

    assert!(token_repository.is_valid_token(&profile.id, &active.token, "2026-04-02T06:22:00Z")?);
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

#[test]
fn export_token_repository_rotates_primary_token_with_grace_window() -> StorageResult<()> {
    let db = Database::open_in_memory()?;
    let profile_repository = ProfileRepository::new(&db);
    let token_repository = ExportTokenRepository::new(&db);
    let profile = sample_profile("profile-rotate-token");
    profile_repository.insert(&profile)?;

    let initial = ExportToken {
        id: "token-initial".to_string(),
        profile_id: profile.id.clone(),
        token: "token-initial-value".to_string(),
        token_type: "primary".to_string(),
        created_at: "2026-04-02T08:00:00Z".to_string(),
        expires_at: None,
    };
    token_repository.insert(&initial)?;

    let rotated = ExportToken {
        id: "token-rotated".to_string(),
        profile_id: profile.id.clone(),
        token: "token-rotated-value".to_string(),
        token_type: "primary".to_string(),
        created_at: "2026-04-02T08:10:00Z".to_string(),
        expires_at: None,
    };
    token_repository.rotate_primary_token_with_grace(
        &profile.id,
        &rotated,
        "2026-04-02T08:20:00Z",
        "2026-04-02T08:10:00Z",
    )?;

    let active = token_repository
        .get_active_token(&profile.id)?
        .expect("轮换后应存在 active token");
    assert_eq!(active.token, rotated.token);
    assert!(token_repository.is_valid_token(
        &profile.id,
        &initial.token,
        "2026-04-02T08:15:00Z"
    )?);
    assert!(!token_repository.is_valid_token(
        &profile.id,
        &initial.token,
        "2026-04-02T08:21:00Z"
    )?);

    let rotated_again = ExportToken {
        id: "token-rotated-again".to_string(),
        profile_id: profile.id.clone(),
        token: "token-rotated-again-value".to_string(),
        token_type: "primary".to_string(),
        created_at: "2026-04-02T08:25:00Z".to_string(),
        expires_at: None,
    };
    token_repository.rotate_primary_token_with_grace(
        &profile.id,
        &rotated_again,
        "2026-04-02T08:35:00Z",
        "2026-04-02T08:25:00Z",
    )?;

    let token_count: i64 = db.with_connection(|connection| {
        let count = connection.query_row(
            "SELECT COUNT(1) FROM export_tokens WHERE profile_id = ?1",
            [profile.id.as_str()],
            |row| row.get(0),
        )?;
        Ok(count)
    })?;
    assert_eq!(token_count, 2, "应清理已过期的旧 token");

    Ok(())
}

#[test]
fn script_log_repository_supports_refresh_job_batch_query() -> StorageResult<()> {
    let db = Database::open_in_memory()?;
    let source_repository = SourceRepository::new(&db);
    let refresh_repository = RefreshJobRepository::new(&db);
    let script_log_repository = ScriptLogRepository::new(&db);

    let source = sample_source("source-script-log", "vendor.example.script");
    source_repository.insert(&source)?;

    let job_a = RefreshJob {
        id: "refresh-job-script-a".to_string(),
        source_instance_id: source.id.clone(),
        trigger_type: "manual".to_string(),
        status: "running".to_string(),
        started_at: Some("2026-04-02T09:00:00Z".to_string()),
        finished_at: None,
        node_count: None,
        error_code: None,
        error_message: None,
    };
    let job_b = RefreshJob {
        id: "refresh-job-script-b".to_string(),
        source_instance_id: source.id.clone(),
        trigger_type: "scheduled".to_string(),
        status: "running".to_string(),
        started_at: Some("2026-04-02T09:05:00Z".to_string()),
        finished_at: None,
        node_count: None,
        error_code: None,
        error_message: None,
    };
    refresh_repository.insert(&job_a)?;
    refresh_repository.insert(&job_b)?;

    for index in 0..3 {
        script_log_repository.insert(&ScriptLog {
            id: format!("script-log-a-{index}"),
            refresh_job_id: job_a.id.clone(),
            source_instance_id: source.id.clone(),
            plugin_id: "vendor.example.script".to_string(),
            level: if index == 0 { "warn" } else { "info" }.to_string(),
            message: format!("A-{index}"),
            created_at: format!("2026-04-02T09:00:0{index}Z"),
        })?;
    }
    script_log_repository.insert(&ScriptLog {
        id: "script-log-b-0".to_string(),
        refresh_job_id: job_b.id.clone(),
        source_instance_id: source.id.clone(),
        plugin_id: "vendor.example.script".to_string(),
        level: "error".to_string(),
        message: "B-0".to_string(),
        created_at: "2026-04-02T09:05:00Z".to_string(),
    })?;

    let logs = script_log_repository.list_by_refresh_job_ids(&[job_a.id.clone(), job_b.id], 2)?;
    assert_eq!(logs.len(), 3, "按 job 限制每个最多 2 条");

    let logs_a = logs
        .iter()
        .filter(|item| item.refresh_job_id == job_a.id)
        .collect::<Vec<_>>();
    assert_eq!(logs_a.len(), 2);
    assert_eq!(logs_a[0].message, "A-0");
    assert_eq!(logs_a[1].message, "A-1");

    let logs_b = logs
        .iter()
        .filter(|item| item.refresh_job_id == "refresh-job-script-b")
        .collect::<Vec<_>>();
    assert_eq!(logs_b.len(), 1);
    assert_eq!(logs_b[0].level, "error");

    Ok(())
}
