use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use app_http_server::{ApiEvent, ServerContext, build_router as build_http_router};
use app_secrets::{
    EnvSecretStore, FileSecretStore, KeyringSecretStore, MemorySecretStore, SecretStore,
};
use app_storage::Database;
use clap::Parser;

use crate::cli::{
    APP_VERSION, CheckArgs, Cli, Command, DEFAULT_DB_FILE_NAME, DEFAULT_HOST, DEFAULT_PORT,
    DEFAULT_SECRETS_FILE_NAME, GuiBootstrap, RefreshArgs, RunArgs, SecretBackendKind,
    SecretStoreArgs,
};
use crate::config::LoadedHeadlessConfig;
use crate::headless::{
    apply_headless_configuration, apply_headless_settings, validate_headless_configuration,
};
use crate::logging::initialize_logging;
use crate::security::{
    acquire_single_instance_lock, ensure_data_dir, is_loopback_host,
    load_or_create_admin_token_with_override, resolve_data_dir, set_owner_only_file_permissions,
};
use crate::settings_seed::seed_default_settings;

pub(crate) async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run(RunArgs {
        config: None,
        host: DEFAULT_HOST.to_string(),
        port: DEFAULT_PORT,
        gui_mode: false,
        data_dir: None,
        secrets: SecretStoreArgs {
            secrets_backend: SecretBackendKind::Keyring,
            secret_key: None,
            secrets_file: None,
        },
    })) {
        Command::Run(args) => run_server(args).await,
        Command::Check(args) => run_check(args),
        Command::Refresh(args) => run_refresh(args),
        Command::Version => {
            println!("subforge-core {APP_VERSION}");
            Ok(())
        }
    }
}

async fn run_server(args: RunArgs) -> Result<()> {
    let loaded_config = load_headless_config(args.config.as_deref())?;
    let (host, port) = resolve_listen_host_port(&args, loaded_config.as_ref())?;
    let data_dir = resolve_data_dir(args.data_dir.clone())?;
    ensure_data_dir(&data_dir)?;
    let lock_file = acquire_single_instance_lock(&data_dir)?;

    let admin_override = loaded_config
        .as_ref()
        .and_then(|config| config.config.server.admin_token.as_deref());
    let admin_token = load_or_create_admin_token_with_override(&data_dir, admin_override)?;

    let database_path = resolve_database_path(&data_dir, loaded_config.as_ref())?;
    let database = initialize_database(&database_path)?;
    let mut effective_secret_args =
        build_effective_secret_args(&args.secrets, loaded_config.as_ref(), &data_dir)?;
    apply_debug_gui_secret_backend_override(&args, &mut effective_secret_args, &data_dir);
    let (secret_backend, secret_store) =
        initialize_secret_store(&effective_secret_args, &data_dir)?;

    seed_default_settings(database.as_ref(), &host, port)?;
    if let Some(config) = &loaded_config {
        println!("已加载无头配置文件: {}", config.path.display());
        apply_headless_settings(config, database.as_ref())?;
        let report = apply_headless_configuration(
            config,
            database.as_ref(),
            Arc::clone(&secret_store),
            &data_dir.join("plugins"),
        )?;
        println!(
            "无头配置已加载：插件安装 {}，来源新增/更新 {}/{}，Profile 新增/更新 {}/{}",
            report.installed_plugins,
            report.created_sources,
            report.updated_sources,
            report.created_profiles,
            report.updated_profiles
        );
    }
    let logging_runtime = initialize_logging(&data_dir, loaded_config.as_ref(), database.as_ref())?;
    if logging_runtime.initialized {
        tracing::info!(
            log_dir = %logging_runtime.log_dir.display(),
            level = %logging_runtime.level,
            retention_days = logging_runtime.retention_days,
            cleaned_files = logging_runtime.cleaned_files,
            "日志系统已初始化"
        );
    }

    if !is_loopback_host(&host) {
        eprintln!(
            "WARNING: 当前监听地址为 {}，这不是回环地址，请确认安全风险。",
            host
        );
        tracing::warn!(listen_host = %host, "监听地址不是回环地址，请确认安全风险");
    }

    let (event_sender, _event_receiver) = tokio::sync::broadcast::channel::<ApiEvent>(256);
    let server_context = ServerContext::new(
        admin_token.clone(),
        data_dir.join("admin_token"),
        Arc::clone(&database),
        Arc::clone(&secret_store),
        data_dir.join("plugins"),
        port,
        event_sender,
    );
    let shutdown_receiver = server_context.shutdown_receiver();
    let app = build_http_router(server_context);

    if args.gui_mode {
        let bootstrap = GuiBootstrap {
            version: APP_VERSION,
            listen_addr: host.clone(),
            port,
            admin_token: admin_token.clone(),
            secrets_backend: secret_backend.as_str(),
        };
        let json = serde_json::to_string(&bootstrap)?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{json}")?;
        stdout.flush()?;
    }

    let socket: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("无效监听地址: {host}:{port}"))?;
    let listener = tokio::net::TcpListener::bind(socket).await?;

    println!(
        "SubForge Core 已启动: http://{}:{}（secrets backend: {}）",
        host,
        port,
        secret_backend.as_str()
    );
    tracing::info!(
        listen_host = %host,
        listen_port = port,
        secret_backend = secret_backend.as_str(),
        "SubForge Core 已启动"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_receiver))
    .await?;

    drop(lock_file);
    drop(logging_runtime);
    Ok(())
}

fn run_check(args: CheckArgs) -> Result<()> {
    let loaded_config = load_headless_config(args.config.as_deref())?;
    let (host, port) = resolve_listen_host_port_for_check(loaded_config.as_ref())?;
    let data_dir = resolve_data_dir(args.data_dir)?;
    ensure_data_dir(&data_dir)?;

    let admin_override = loaded_config
        .as_ref()
        .and_then(|config| config.config.server.admin_token.as_deref());
    load_or_create_admin_token_with_override(&data_dir, admin_override)?;

    let database_path = resolve_database_path(&data_dir, loaded_config.as_ref())?;
    let database = initialize_database(&database_path)?;
    let effective_secret_args =
        build_effective_secret_args(&args.secrets, loaded_config.as_ref(), &data_dir)?;
    initialize_secret_store(&effective_secret_args, &data_dir)?;
    seed_default_settings(database.as_ref(), &host, port)?;

    if let Some(config) = &loaded_config {
        println!("已加载无头配置文件: {}", config.path.display());
        apply_headless_settings(config, database.as_ref())?;
        validate_headless_configuration(config)?;
    }

    println!(
        "配置检查通过，数据目录: {}，密钥后端: {}",
        data_dir.display(),
        effective_secret_args.secrets_backend.as_str()
    );
    Ok(())
}

pub(crate) fn run_refresh(args: RefreshArgs) -> Result<()> {
    if let Some(source_id) = args.source_id {
        println!("收到手动刷新请求，来源: {source_id}（功能将在后续阶段实现）");
    } else {
        println!("收到全量刷新请求（功能将在后续阶段实现）");
    }
    Ok(())
}

async fn shutdown_signal(mut shutdown_receiver: tokio::sync::watch::Receiver<bool>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("WARNING: 注册 SIGTERM 监听失败: {err:#}");
                tracing::warn!(error = %err, "注册 SIGTERM 监听失败，退回 Ctrl+C 监听");
                let _ = tokio::signal::ctrl_c().await;
                println!("收到退出信号，正在优雅关闭...");
                tracing::info!("收到退出信号，正在优雅关闭");
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
            _ = shutdown_receiver.changed() => {},
        }
    }

    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown_receiver.changed() => {},
        }
    }

    println!("收到退出信号，正在优雅关闭...");
    tracing::info!("收到退出信号，正在优雅关闭");
}

fn load_headless_config(path: Option<&Path>) -> Result<Option<LoadedHeadlessConfig>> {
    path.map(LoadedHeadlessConfig::from_file).transpose()
}

fn resolve_listen_host_port(
    args: &RunArgs,
    loaded_config: Option<&LoadedHeadlessConfig>,
) -> Result<(String, u16)> {
    if let Some(config) = loaded_config {
        return config.listen_host_port();
    }
    Ok((args.host.clone(), args.port))
}

fn resolve_listen_host_port_for_check(
    loaded_config: Option<&LoadedHeadlessConfig>,
) -> Result<(String, u16)> {
    if let Some(config) = loaded_config {
        return config.listen_host_port();
    }
    Ok((DEFAULT_HOST.to_string(), DEFAULT_PORT))
}

fn resolve_database_path(
    data_dir: &Path,
    loaded_config: Option<&LoadedHeadlessConfig>,
) -> Result<PathBuf> {
    if let Some(config) = loaded_config {
        if let Some(path) = config.resolved_db_path()? {
            return Ok(path);
        }
    }
    Ok(data_dir.join(DEFAULT_DB_FILE_NAME))
}

fn build_effective_secret_args(
    cli_args: &SecretStoreArgs,
    loaded_config: Option<&LoadedHeadlessConfig>,
    data_dir: &Path,
) -> Result<SecretStoreArgs> {
    let mut effective = cli_args.clone();
    if let Some(config) = loaded_config {
        if let Some(backend) = config.backend_override() {
            effective.secrets_backend = backend;
        }
        if let Some(secrets_file) = config.resolved_secrets_file_path()? {
            effective.secrets_file = Some(secrets_file);
        }
    }
    if effective.secrets_backend == SecretBackendKind::File && effective.secrets_file.is_none() {
        effective.secrets_file = Some(data_dir.join(DEFAULT_SECRETS_FILE_NAME));
    }
    Ok(effective)
}

fn apply_debug_gui_secret_backend_override(
    args: &RunArgs,
    effective: &mut SecretStoreArgs,
    data_dir: &Path,
) {
    if !cfg!(debug_assertions) || !args.gui_mode {
        return;
    }
    if effective.secrets_backend != SecretBackendKind::Keyring {
        return;
    }

    effective.secrets_backend = SecretBackendKind::File;
    if effective.secret_key.is_none() {
        effective.secret_key = std::env::var("SUBFORGE_SECRET_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some("subforge-desktop-dev-secret-key".to_string()));
    }
    if effective.secrets_file.is_none() {
        effective.secrets_file = Some(data_dir.join(DEFAULT_SECRETS_FILE_NAME));
    }
}

fn initialize_database(database_path: &Path) -> Result<Arc<Database>> {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建数据库目录失败: {}", parent.display()))?;
    }
    let database = Database::open(database_path)
        .with_context(|| format!("初始化数据库失败: {}", database_path.display()))?;
    set_owner_only_file_permissions(database_path)?;
    Ok(Arc::new(database))
}

fn initialize_secret_store(
    args: &SecretStoreArgs,
    data_dir: &Path,
) -> Result<(SecretBackendKind, Arc<dyn SecretStore>)> {
    let backend = args.secrets_backend;
    let store: Arc<dyn SecretStore> = match backend {
        SecretBackendKind::Keyring => Arc::new(KeyringSecretStore::new()),
        SecretBackendKind::Env => Arc::new(EnvSecretStore::new()),
        SecretBackendKind::Memory => Arc::new(MemorySecretStore::new()),
        SecretBackendKind::File => {
            let secret_key = args
                .secret_key
                .clone()
                .or_else(|| std::env::var("SUBFORGE_SECRET_KEY").ok())
                .ok_or_else(|| {
                    anyhow!("file 密钥后端需要 --secret-key 或环境变量 SUBFORGE_SECRET_KEY")
                })?;
            let secrets_file = args
                .secrets_file
                .clone()
                .unwrap_or_else(|| data_dir.join(DEFAULT_SECRETS_FILE_NAME));
            let store = FileSecretStore::new(&secrets_file, secret_key)
                .with_context(|| format!("初始化 file 密钥后端失败: {}", secrets_file.display()))?;
            Arc::new(store)
        }
    };

    Ok((backend, store))
}
