use super::*;

#[tokio::test]
async fn engine_refresh_source_uses_profile_headers_from_plugin_manifest() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-profile-routing");
    let plugins_dir = temp_root.join("plugins");
    let install_service = PluginInstallService::new(&db, &plugins_dir);
    let standard_plugin_dir = create_static_plugin_with_network_profile(
        &temp_root,
        "standard-plugin",
        "vendor.example.profile-standard",
        "standard",
    );
    let chrome_plugin_dir = create_static_plugin_with_network_profile(
        &temp_root,
        "chrome-plugin",
        "vendor.example.profile-browser-chrome",
        "browser_chrome",
    );
    install_service
        .install_from_dir(&standard_plugin_dir)
        .expect("安装 standard 插件应成功");
    install_service
        .install_from_dir(&chrome_plugin_dir)
        .expect("安装 browser_chrome 插件应成功");

    let (url, total_requests, chrome_requests, server_task) = start_profile_gate_server(
        "/sub",
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let source_service = SourceService::new(&db, &plugins_dir, secret_store.as_ref());
    let mut standard_config = BTreeMap::new();
    standard_config.insert("url".to_string(), json!(format!("{url}/sub")));
    let standard_source = source_service
        .create_source(
            "vendor.example.profile-standard",
            "Standard Profile Source",
            standard_config,
        )
        .expect("创建 standard 来源应成功");

    let mut chrome_config = BTreeMap::new();
    chrome_config.insert("url".to_string(), json!(format!("{url}/sub")));
    let chrome_source = source_service
        .create_source(
            "vendor.example.profile-browser-chrome",
            "Browser Chrome Source",
            chrome_config,
        )
        .expect("创建 browser_chrome 来源应成功");

    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let standard_error = engine
        .refresh_source(&standard_source.source.id, "manual")
        .await
        .expect_err("standard 档位不应通过 Chrome Header 校验");
    assert!(matches!(standard_error, CoreError::SubscriptionFetch(_)));

    let chrome_result = engine
        .refresh_source(&chrome_source.source.id, "manual")
        .await
        .expect("browser_chrome 档位应通过 Header 校验");
    assert_eq!(chrome_result.node_count, 3);
    assert_eq!(total_requests.load(Ordering::SeqCst), 2);
    assert_eq!(chrome_requests.load(Ordering::SeqCst), 1);

    server_task.abort();
    cleanup_dir(&temp_root);
}

#[tokio::test]
async fn engine_refresh_source_records_refresh_job_success() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-refresh");
    let plugins_dir = temp_root.join("plugins");
    let install_service = PluginInstallService::new(&db, &plugins_dir);
    install_service
        .install_from_dir(builtins_static_plugin_dir())
        .expect("安装内置插件应成功");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let source_service = SourceService::new(&db, &plugins_dir, secret_store.as_ref());
    let (url, server_task) = start_fixture_server(
        "/sub",
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;
    let mut config = BTreeMap::new();
    config.insert("url".to_string(), json!(format!("{url}/sub")));
    let source = source_service
        .create_source("subforge.builtin.static", "Engine Source", config)
        .expect("创建来源应成功");

    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let refresh_result = engine
        .refresh_source(&source.source.id, "manual")
        .await
        .expect("刷新应成功");
    assert_eq!(refresh_result.source_id, source.source.id);
    assert_eq!(refresh_result.node_count, 3);

    let refresh_repository = RefreshJobRepository::new(&db);
    let jobs = refresh_repository
        .list_by_source(&source.source.id)
        .expect("读取 refresh_jobs 失败");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, refresh_result.refresh_job_id);
    assert_eq!(jobs[0].status, "success");
    assert_eq!(jobs[0].node_count, Some(3));
    assert!(jobs[0].error_code.is_none());

    server_task.abort();
    cleanup_dir(&temp_root);
}

#[tokio::test]
async fn engine_refresh_source_executes_script_pipeline_and_persists_state() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-script-pipeline");
    let plugins_dir = temp_root.join("plugins");
    let script_plugin_dir = create_script_plugin_dir(
        &temp_root,
        "script-plugin",
        "vendor.example.script-pipeline",
        Some(
            r#"
                function login(ctx, config, state)
                    if ctx.has_state then
                        return { ok = false, error = "login should be skipped when state exists" }
                    end
                    return {
                        ok = true,
                        state = {
                            session = "session-" .. config.seed,
                            counter = 1,
                            stage = "login"
                        }
                    }
                end
            "#,
        ),
        Some(
            r#"
                function refresh(ctx, config, state)
                    if state == nil then
                        return { ok = false, error = "state missing in refresh" }
                    end
                    return {
                        ok = true,
                        state = {
                            session = state.session,
                            counter = (state.counter or 0) + 1,
                            stage = "refresh"
                        }
                    }
                end
            "#,
        ),
        r#"
            function fetch(ctx, config, state)
                if state == nil then
                    return { ok = false, error = "state missing in fetch" }
                end
                local counter = (state.counter or 0) + 1
                local node_name = "script-" .. tostring(counter)
                local content = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.com:443#" .. node_name
                log.info("fetch token=abc123 stage=fetch")
                return {
                    ok = true,
                    subscription = { content = content },
                    state = {
                        session = state.session,
                        counter = counter,
                        stage = "fetch"
                    }
                }
            end
        "#,
    );
    let install_service = PluginInstallService::new(&db, &plugins_dir);
    install_service
        .install_from_dir(&script_plugin_dir)
        .expect("安装脚本插件应成功");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let source_service = SourceService::new(&db, &plugins_dir, secret_store.as_ref());
    let mut config = BTreeMap::new();
    config.insert("seed".to_string(), json!("alpha"));
    let source = source_service
        .create_source(
            "vendor.example.script-pipeline",
            "Script Pipeline Source",
            config,
        )
        .expect("创建脚本来源应成功");

    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let first_run = engine
        .refresh_source(&source.source.id, "manual")
        .await
        .expect("首次脚本刷新应成功");
    assert_eq!(first_run.node_count, 1);

    let source_repository = SourceRepository::new(&db);
    let first_state_raw = source_repository
        .get_by_id(&source.source.id)
        .expect("读取来源失败")
        .and_then(|item| item.state_json)
        .expect("首次刷新后应持久化 state_json");
    let first_state: Value =
        serde_json::from_str(&first_state_raw).expect("state_json 应为合法 JSON");
    assert_eq!(first_state["counter"], json!(3));
    assert_eq!(first_state["stage"], json!("fetch"));
    assert_eq!(first_state["session"], json!("session-alpha"));

    let second_run = engine
        .refresh_source(&source.source.id, "manual")
        .await
        .expect("第二次脚本刷新应成功");
    assert_eq!(second_run.node_count, 1);

    let second_state_raw = source_repository
        .get_by_id(&source.source.id)
        .expect("读取来源失败")
        .and_then(|item| item.state_json)
        .expect("第二次刷新后应持久化 state_json");
    let second_state: Value =
        serde_json::from_str(&second_state_raw).expect("state_json 应为合法 JSON");
    assert_eq!(second_state["counter"], json!(5));
    assert_eq!(second_state["stage"], json!("fetch"));
    assert_eq!(second_state["session"], json!("session-alpha"));

    let refresh_repository = RefreshJobRepository::new(&db);
    let jobs = refresh_repository
        .list_by_source(&source.source.id)
        .expect("读取 refresh_jobs 失败");
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.status == "success"));

    let script_log_repository = ScriptLogRepository::new(&db);
    let script_logs = script_log_repository
        .list_by_refresh_job_ids(
            &[
                first_run.refresh_job_id.clone(),
                second_run.refresh_job_id.clone(),
            ],
            10,
        )
        .expect("读取脚本日志失败");
    assert_eq!(script_logs.len(), 2);
    assert!(
        script_logs
            .iter()
            .all(|log| log.source_instance_id == source.source.id)
    );
    assert!(
        script_logs
            .iter()
            .all(|log| log.message.contains("token=***")),
        "脚本日志中的敏感键值应被脱敏"
    );
    assert!(
        script_logs
            .iter()
            .all(|log| !log.message.contains("abc123"))
    );

    cleanup_dir(&temp_root);
}

#[tokio::test]
async fn engine_refresh_source_stops_when_script_login_fails() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-script-login-failed");
    let plugins_dir = temp_root.join("plugins");
    let script_plugin_dir = create_script_plugin_dir(
        &temp_root,
        "script-login-failed-plugin",
        "vendor.example.script-login-failed",
        Some(
            r#"
                function login(ctx, config, state)
                    return { ok = false, error = "invalid credentials" }
                end
            "#,
        ),
        Some(
            r#"
                function refresh(ctx, config, state)
                    return { ok = true, state = { phase = "refresh" } }
                end
            "#,
        ),
        r#"
            function fetch(ctx, config, state)
                return {
                    ok = true,
                    subscription = {
                        content = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.com:443#unexpected-fetch"
                    }
                }
            end
        "#,
    );
    let install_service = PluginInstallService::new(&db, &plugins_dir);
    install_service
        .install_from_dir(&script_plugin_dir)
        .expect("安装脚本插件应成功");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let source_service = SourceService::new(&db, &plugins_dir, secret_store.as_ref());
    let mut config = BTreeMap::new();
    config.insert("seed".to_string(), json!("beta"));
    let source = source_service
        .create_source(
            "vendor.example.script-login-failed",
            "Script Login Failed Source",
            config,
        )
        .expect("创建脚本来源应成功");

    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let error = engine
        .refresh_source(&source.source.id, "manual")
        .await
        .expect_err("login 失败时刷新应失败");
    assert_eq!(error.code(), "E_SCRIPT_RUNTIME");
    assert!(matches!(
        error,
        CoreError::PluginRuntime(app_plugin_runtime::PluginRuntimeError::ScriptRuntime(_))
    ));

    let cache_repository = NodeCacheRepository::new(&db);
    assert!(
        cache_repository
            .get_by_source(&source.source.id)
            .expect("查询 node_cache 失败")
            .is_none(),
        "login 失败后不应继续 fetch 并写入 node_cache"
    );

    let source_repository = SourceRepository::new(&db);
    let state = source_repository
        .get_by_id(&source.source.id)
        .expect("读取来源失败")
        .and_then(|item| item.state_json);
    assert!(state.is_none(), "login 失败不应写入 state_json");

    let refresh_repository = RefreshJobRepository::new(&db);
    let jobs = refresh_repository
        .list_by_source(&source.source.id)
        .expect("读取 refresh_jobs 失败");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, "failed");
    assert_eq!(jobs[0].error_code.as_deref(), Some("E_SCRIPT_RUNTIME"));

    cleanup_dir(&temp_root);
}

#[tokio::test]
async fn engine_refresh_source_supports_structured_script_error() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-script-login-structured-error");
    let plugins_dir = temp_root.join("plugins");
    let script_plugin_dir = create_script_plugin_dir(
        &temp_root,
        "script-login-structured-error-plugin",
        "vendor.example.script-login-structured-error",
        Some(
            r#"
                function login(ctx, config, state)
                    return {
                        ok = false,
                        error = {
                            code = "E_LOGIN_FAILED",
                            message = "账号或密码错误",
                            retryable = false
                        }
                    }
                end
            "#,
        ),
        Some(
            r#"
                function refresh(ctx, config, state)
                    return { ok = true, state = { phase = "refresh" } }
                end
            "#,
        ),
        r#"
            function fetch(ctx, config, state)
                return {
                    ok = true,
                    subscription = {
                        content = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.com:443#unexpected-fetch"
                    }
                }
            end
        "#,
    );
    let install_service = PluginInstallService::new(&db, &plugins_dir);
    install_service
        .install_from_dir(&script_plugin_dir)
        .expect("安装脚本插件应成功");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let source_service = SourceService::new(&db, &plugins_dir, secret_store.as_ref());
    let mut config = BTreeMap::new();
    config.insert("seed".to_string(), json!("gamma"));
    let source = source_service
        .create_source(
            "vendor.example.script-login-structured-error",
            "Script Login Structured Error Source",
            config,
        )
        .expect("创建脚本来源应成功");

    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let error = engine
        .refresh_source(&source.source.id, "manual")
        .await
        .expect_err("login 返回结构化错误时刷新应失败");
    assert_eq!(error.code(), "E_SCRIPT_RUNTIME");
    let message = error.to_string();
    assert!(message.contains("login 失败"));
    assert!(message.contains("E_LOGIN_FAILED"));
    assert!(message.contains("账号或密码错误"));
    assert!(message.contains("retryable=false"));

    cleanup_dir(&temp_root);
}

#[test]
fn engine_ensure_profile_export_token_is_idempotent() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-token");
    let plugins_dir = temp_root.join("plugins");
    fs::create_dir_all(&plugins_dir).expect("创建插件目录失败");
    let profile_repository = app_storage::ProfileRepository::new(&db);
    let profile = app_common::Profile {
        id: "profile-engine-token".to_string(),
        name: "Engine Token".to_string(),
        description: None,
        created_at: "2026-04-02T07:00:00Z".to_string(),
        updated_at: "2026-04-02T07:00:00Z".to_string(),
    };
    profile_repository
        .insert(&profile)
        .expect("写入 profile 失败");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let token_a = engine
        .ensure_profile_export_token(&profile.id)
        .expect("首次生成 token 应成功");
    let token_b = engine
        .ensure_profile_export_token(&profile.id)
        .expect("重复生成应返回已有 token");
    assert_eq!(token_a, token_b);
    assert_eq!(token_a.len(), 43);

    let token_repository = ExportTokenRepository::new(&db);
    let stored = token_repository
        .get_active_token(&profile.id)
        .expect("读取 active token 失败")
        .expect("应存在 active token");
    assert_eq!(stored.token, token_a);

    cleanup_dir(&temp_root);
}

#[test]
fn engine_rotate_profile_export_token_keeps_grace_window() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let temp_root = create_temp_dir("engine-token-rotate");
    let plugins_dir = temp_root.join("plugins");
    fs::create_dir_all(&plugins_dir).expect("创建插件目录失败");
    let profile_repository = app_storage::ProfileRepository::new(&db);
    let profile = app_common::Profile {
        id: "profile-engine-token-rotate".to_string(),
        name: "Engine Token Rotate".to_string(),
        description: None,
        created_at: "2026-04-02T07:30:00Z".to_string(),
        updated_at: "2026-04-02T07:30:00Z".to_string(),
    };
    profile_repository
        .insert(&profile)
        .expect("写入 profile 失败");

    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let engine = Engine::new(&db, &plugins_dir, Arc::clone(&secret_store));
    let original = engine
        .ensure_profile_export_token(&profile.id)
        .expect("初始化 token 应成功");
    let rotated = engine
        .rotate_profile_export_token(&profile.id)
        .expect("轮换 token 应成功");

    assert_ne!(original, rotated.token);
    assert_eq!(rotated.token.len(), 43);

    let repository = ExportTokenRepository::new(&db);
    let active = repository
        .get_active_token(&profile.id)
        .expect("读取 active token 失败")
        .expect("轮换后应有 active token");
    assert_eq!(active.token, rotated.token);
    assert!(
        repository
            .is_valid_token(&profile.id, &original, "1970-01-01T00:00:00Z")
            .expect("校验旧 token 失败")
    );
    assert!(
        !repository
            .is_valid_token(&profile.id, &original, &rotated.grace_expires_at)
            .expect("校验旧 token 过期失败")
    );

    cleanup_dir(&temp_root);
}
