use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use app_common::{AppSetting, ErrorResponse};
use app_secrets::{
    EnvSecretStore, FileSecretStore, KeyringSecretStore, MemorySecretStore, SecretStore,
};
use app_storage::{Database, SettingsRepository, StorageError};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header::AUTHORIZATION, header::HOST};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::{Args, Parser, Subcommand, ValueEnum};
use fs2::FileExt;
use rand::Rng;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 18118;
const DEFAULT_DB_FILE_NAME: &str = "subforge.db";
const DEFAULT_SECRETS_FILE_NAME: &str = "secrets.enc";

#[derive(Parser, Debug)]
#[command(name = "subforge-core", version = APP_VERSION, about = "SubForge Core 守护进程")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 启动 Core 服务
    Run(RunArgs),
    /// 检查运行配置
    Check(CheckArgs),
    /// 手动触发刷新（占位）
    Refresh(RefreshArgs),
    /// 输出版本信息
    Version,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum SecretBackendKind {
    Keyring,
    Env,
    File,
    Memory,
}

impl SecretBackendKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keyring => "keyring",
            Self::Env => "env",
            Self::File => "file",
            Self::Memory => "memory",
        }
    }
}

#[derive(Args, Debug, Clone)]
struct SecretStoreArgs {
    /// 密钥后端（GUI 默认 keyring）
    #[arg(long, value_enum, default_value_t = SecretBackendKind::Keyring)]
    secrets_backend: SecretBackendKind,
    /// file 后端主密码（也可通过 SUBFORGE_SECRET_KEY 传入）
    #[arg(long)]
    secret_key: Option<String>,
    /// file 后端密钥文件路径，默认 {data_dir}/secrets.enc
    #[arg(long)]
    secrets_file: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct RunArgs {
    /// 监听地址，默认仅本机回环
    #[arg(long, default_value = DEFAULT_HOST)]
    host: String,
    /// 监听端口
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// GUI 模式，首行输出启动信息 JSON
    #[arg(long, default_value_t = false)]
    gui_mode: bool,
    /// 数据目录，默认当前目录下 .subforge
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(flatten)]
    secrets: SecretStoreArgs,
}

#[derive(Args, Debug, Clone)]
struct CheckArgs {
    /// 数据目录，默认当前目录下 .subforge
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(flatten)]
    secrets: SecretStoreArgs,
}

#[derive(Args, Debug)]
struct RefreshArgs {
    /// 来源 ID（预留）
    #[arg(long)]
    source_id: Option<String>,
}

#[derive(Debug, Clone)]
struct HostValidationState {
    allowed_hosts: Arc<HashSet<String>>,
}

impl HostValidationState {
    fn new(port: u16) -> Self {
        let mut hosts = HashSet::new();
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            hosts.insert(host.to_string());
            hosts.insert(format!("{host}:{port}"));
        }

        Self {
            allowed_hosts: Arc::new(hosts),
        }
    }

    fn is_allowed(&self, host_header: &str) -> bool {
        self.allowed_hosts.contains(host_header)
    }
}

#[derive(Clone)]
struct AppState {
    admin_token: Arc<String>,
    database: Arc<Database>,
    _secret_backend: SecretBackendKind,
    _secret_store: Arc<dyn SecretStore>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
struct GuiBootstrap {
    version: &'static str,
    listen_addr: String,
    port: u16,
    admin_token: String,
    secrets_backend: &'static str,
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSettingsRequest {
    settings: BTreeMap<String, String>,
}

type ApiResult<T> =
    std::result::Result<(StatusCode, axum::Json<T>), (StatusCode, axum::Json<ErrorResponse>)>;

#[tokio::main]
async fn main() {
    if let Err(err) = run_cli().await {
        eprintln!("错误: {err:#}");
        std::process::exit(1);
    }
}

async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run(RunArgs {
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
    let data_dir = resolve_data_dir(args.data_dir.clone())?;
    ensure_data_dir(&data_dir)?;
    let lock_file = acquire_single_instance_lock(&data_dir)?;
    let admin_token = load_or_create_admin_token(&data_dir)?;
    let database = initialize_database(&data_dir)?;
    let (secret_backend, secret_store) = initialize_secret_store(&args.secrets, &data_dir)?;

    seed_default_settings(database.as_ref(), &args)?;

    if !is_loopback_host(&args.host) {
        eprintln!(
            "WARNING: 当前监听地址为 {}，这不是回环地址，请确认安全风险。",
            args.host
        );
    }

    let app_state = AppState {
        admin_token: Arc::new(admin_token.clone()),
        database,
        _secret_backend: secret_backend,
        _secret_store: secret_store,
    };
    let host_state = HostValidationState::new(args.port);

    let api_router = Router::new()
        .route(
            "/system/settings",
            get(get_system_settings_handler).put(update_system_settings_handler),
        )
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            admin_auth_middleware,
        ))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/health", get(health_handler))
        .nest("/api", api_router)
        .layer(middleware::from_fn_with_state(
            host_state,
            host_validation_middleware,
        ));

    if args.gui_mode {
        let bootstrap = GuiBootstrap {
            version: APP_VERSION,
            listen_addr: args.host.clone(),
            port: args.port,
            admin_token: admin_token.clone(),
            secrets_backend: secret_backend.as_str(),
        };
        let json = serde_json::to_string(&bootstrap)?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{json}")?;
        stdout.flush()?;
    }

    let socket: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("无效监听地址: {}:{}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(socket).await?;

    println!(
        "SubForge Core 已启动: http://{}:{}（secrets backend: {}）",
        args.host,
        args.port,
        secret_backend.as_str()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    drop(lock_file);
    Ok(())
}

fn run_check(args: CheckArgs) -> Result<()> {
    let data_dir = resolve_data_dir(args.data_dir)?;
    ensure_data_dir(&data_dir)?;
    load_or_create_admin_token(&data_dir)?;
    let database = initialize_database(&data_dir)?;
    initialize_secret_store(&args.secrets, &data_dir)?;
    seed_default_settings(
        database.as_ref(),
        &RunArgs {
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
            gui_mode: false,
            data_dir: Some(data_dir.clone()),
            secrets: args.secrets.clone(),
        },
    )?;
    println!(
        "配置检查通过，数据目录: {}，密钥后端: {}",
        data_dir.display(),
        args.secrets.secrets_backend.as_str()
    );
    Ok(())
}

fn run_refresh(args: RefreshArgs) -> Result<()> {
    if let Some(source_id) = args.source_id {
        println!("收到手动刷新请求，来源: {source_id}（功能将在后续阶段实现）");
    } else {
        println!("收到全量刷新请求（功能将在后续阶段实现）");
    }
    Ok(())
}

async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(HealthResponse {
            status: "ok",
            version: APP_VERSION,
        }),
    )
}

async fn get_system_settings_handler(State(state): State<AppState>) -> ApiResult<SettingsResponse> {
    let repository = SettingsRepository::new(state.database.as_ref());
    let settings = repository.get_all().map_err(storage_error_to_response)?;

    Ok((
        StatusCode::OK,
        axum::Json(SettingsResponse {
            settings: map_settings(settings),
        }),
    ))
}

async fn update_system_settings_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<UpdateSettingsRequest>,
) -> ApiResult<SettingsResponse> {
    if payload.settings.is_empty() {
        return Err(config_error_response("请求体 settings 不能为空"));
    }

    let updated_at = current_timestamp_rfc3339().map_err(|error| {
        eprintln!("ERROR: 生成时间戳失败: {error:#}");
        internal_error_response()
    })?;

    let repository = SettingsRepository::new(state.database.as_ref());
    for (key, value) in payload.settings {
        if key.trim().is_empty() {
            return Err(config_error_response("设置键不能为空"));
        }

        repository
            .set(&AppSetting {
                key,
                value,
                updated_at: updated_at.clone(),
            })
            .map_err(storage_error_to_response)?;
    }

    let settings = repository.get_all().map_err(storage_error_to_response)?;
    Ok((
        StatusCode::OK,
        axum::Json(SettingsResponse {
            settings: map_settings(settings),
        }),
    ))
}

async fn admin_auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let valid = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_bearer_token)
        .is_some_and(|token| token == state.admin_token.as_str());

    if !valid {
        return unauthorized_error_response().into_response();
    }

    next.run(request).await
}

async fn host_validation_middleware(
    State(state): State<HostValidationState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_host)
        .unwrap_or_default();

    if !state.is_allowed(&host) {
        let body = ErrorResponse::new("E_AUTH", "Forbidden: invalid Host header", false);
        return (StatusCode::FORBIDDEN, axum::Json(body)).into_response();
    }

    next.run(request).await
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("WARNING: 注册 SIGTERM 监听失败: {err:#}");
                let _ = tokio::signal::ctrl_c().await;
                println!("收到退出信号，正在优雅关闭...");
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    println!("收到退出信号，正在优雅关闭...");
}

fn resolve_data_dir(data_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = data_dir {
        return Ok(path);
    }
    let cwd = std::env::current_dir().context("读取当前目录失败")?;
    Ok(cwd.join(".subforge"))
}

fn ensure_data_dir(data_dir: &Path) -> Result<()> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("创建数据目录失败: {}", data_dir.display()))?;
    set_owner_only_directory_permissions(data_dir)?;
    Ok(())
}

fn initialize_database(data_dir: &Path) -> Result<Arc<Database>> {
    let database_path = data_dir.join(DEFAULT_DB_FILE_NAME);
    let database = Database::open(&database_path)
        .with_context(|| format!("初始化数据库失败: {}", database_path.display()))?;
    set_owner_only_file_permissions(&database_path)?;
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

fn load_or_create_admin_token(data_dir: &Path) -> Result<String> {
    let token_path = data_dir.join("admin_token");
    if token_path.exists() {
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("读取 admin_token 失败: {}", token_path.display()))?;
        let token = token.trim().to_string();
        if !token.is_empty() {
            set_owner_only_file_permissions(&token_path)?;
            return Ok(token);
        }
    }

    let token = generate_admin_token();
    fs::write(&token_path, format!("{token}\n"))
        .with_context(|| format!("写入 admin_token 失败: {}", token_path.display()))?;
    set_owner_only_file_permissions(&token_path)?;
    Ok(token)
}

fn acquire_single_instance_lock(data_dir: &Path) -> Result<File> {
    let lock_path = data_dir.join("subforge.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("创建锁文件失败: {}", lock_path.display()))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("另一个 Core 实例已在运行"))?;
    Ok(lock_file)
}

fn generate_admin_token() -> String {
    let mut bytes = [0_u8; 32];
    let mut rng = rand::rng();
    rng.fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn seed_default_settings(database: &Database, args: &RunArgs) -> Result<()> {
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
    ];

    for (key, value) in defaults {
        set_default_setting_if_absent(&repository, key, value, &updated_at)?;
    }

    Ok(())
}

fn set_default_setting_if_absent(
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

fn map_settings(settings: Vec<AppSetting>) -> BTreeMap<String, String> {
    settings
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect()
}

fn parse_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    let token = trimmed.strip_prefix("Bearer ")?;
    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}

fn normalize_host(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        normalize_host(host).as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn unauthorized_error_response() -> (StatusCode, axum::Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(ErrorResponse::new("E_AUTH", "Unauthorized", false)),
    )
}

fn config_error_response(message: &str) -> (StatusCode, axum::Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(ErrorResponse::new("E_CONFIG_INVALID", message, false)),
    )
}

fn internal_error_response() -> (StatusCode, axum::Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(ErrorResponse::new(
            "E_INTERNAL",
            "Internal server error",
            true,
        )),
    )
}

fn storage_error_to_response(error: StorageError) -> (StatusCode, axum::Json<ErrorResponse>) {
    eprintln!("ERROR: 存储层操作失败: {error:#}");
    internal_error_response()
}

fn set_owner_only_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("设置文件权限失败: {}", path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(path, false)?;
    }

    Ok(())
}

fn set_owner_only_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("设置目录权限失败: {}", path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(path, true)?;
    }

    Ok(())
}

#[cfg(windows)]
fn apply_windows_owner_only_acl(path: &Path, is_directory: bool) -> Result<()> {
    let username = std::env::var("USERNAME").context("读取当前用户名失败，无法设置 ACL")?;
    let target = path.to_string_lossy().into_owned();
    let permission = if is_directory { "(OI)(CI)F" } else { "(R,W)" };
    let grant = format!("{username}:{permission}");

    run_icacls(&target, &["/inheritance:r"])?;
    run_icacls(&target, &["/grant:r", &grant])?;

    Ok(())
}

#[cfg(windows)]
fn run_icacls(target: &str, args: &[&str]) -> Result<()> {
    use std::process::Command;

    let output = Command::new("icacls")
        .arg(target)
        .args(args)
        .output()
        .with_context(|| format!("执行 icacls 失败: icacls {target} {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "icacls 执行失败: icacls {target} {}，stdout: {}，stderr: {}",
            args.join(" "),
            stdout.trim(),
            stderr.trim()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        HostValidationState, parse_bearer_token, run_refresh, set_default_setting_if_absent,
    };
    use app_common::AppSetting;
    use app_storage::{Database, SettingsRepository, StorageResult};

    #[test]
    fn host_validation_allows_loopback_hosts_for_custom_port() {
        let state = HostValidationState::new(19123);

        assert!(state.is_allowed("127.0.0.1"));
        assert!(state.is_allowed("127.0.0.1:19123"));
        assert!(state.is_allowed("localhost:19123"));
        assert!(state.is_allowed("[::1]:19123"));
    }

    #[test]
    fn host_validation_rejects_non_loopback_hosts() {
        let state = HostValidationState::new(18118);

        assert!(!state.is_allowed("0.0.0.0"));
        assert!(!state.is_allowed("0.0.0.0:18118"));
        assert!(!state.is_allowed("evil.com"));
    }

    #[test]
    fn bearer_token_parser_works() {
        assert_eq!(parse_bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(parse_bearer_token("Bearer    abc"), Some("abc"));
        assert_eq!(parse_bearer_token("bearer abc"), None);
        assert_eq!(parse_bearer_token("Bearer "), None);
        assert_eq!(parse_bearer_token("abc"), None);
    }

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
        run_refresh(super::RefreshArgs { source_id: None }).expect("全量刷新占位应返回成功");
        run_refresh(super::RefreshArgs {
            source_id: Some("source-1".to_string()),
        })
        .expect("单来源刷新占位应返回成功");
    }
}
