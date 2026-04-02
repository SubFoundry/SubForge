use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use app_common::ErrorResponse;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header::HOST};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::{Args, Parser, Subcommand};
use fs2::FileExt;
use rand::Rng;
use serde::Serialize;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 18118;

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

#[derive(Args, Debug)]
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
}

#[derive(Args, Debug)]
struct CheckArgs {
    /// 数据目录，默认当前目录下 .subforge
    #[arg(long)]
    data_dir: Option<PathBuf>,
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
}

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
    let data_dir = resolve_data_dir(args.data_dir)?;
    ensure_data_dir(&data_dir)?;
    let lock_file = acquire_single_instance_lock(&data_dir)?;
    let admin_token = load_or_create_admin_token(&data_dir)?;

    if !is_loopback_host(&args.host) {
        eprintln!(
            "WARNING: 当前监听地址为 {}，这不是回环地址，请确认安全风险。",
            args.host
        );
    }

    let host_state = HostValidationState::new(args.port);
    let app =
        Router::new()
            .route("/health", get(health_handler))
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

    println!("SubForge Core 已启动: http://{}:{}", args.host, args.port);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    drop(lock_file);
    Ok(())
}

fn run_check(args: CheckArgs) -> Result<()> {
    let data_dir = resolve_data_dir(args.data_dir)?;
    ensure_data_dir(&data_dir)?;
    println!("配置检查通过，数据目录: {}", data_dir.display());
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

fn normalize_host(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        normalize_host(host).as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn set_owner_only_file_permissions(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(_path, permissions)
            .with_context(|| format!("设置文件权限失败: {}", _path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(_path, false)?;
    }

    Ok(())
}

fn set_owner_only_directory_permissions(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(_path, permissions)
            .with_context(|| format!("设置目录权限失败: {}", _path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(_path, true)?;
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
    use super::HostValidationState;

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
}
