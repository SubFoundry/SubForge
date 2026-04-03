use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";
pub(crate) const DEFAULT_PORT: u16 = 18118;
pub(crate) const DEFAULT_DB_FILE_NAME: &str = "subforge.db";
pub(crate) const DEFAULT_SECRETS_FILE_NAME: &str = "secrets.enc";

#[derive(Parser, Debug)]
#[command(name = "subforge-core", version = APP_VERSION, about = "SubForge Core 守护进程")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// 启动 Core 服务
    Run(RunArgs),
    /// 检查运行配置
    Check(CheckArgs),
    /// 手动触发刷新（占位）
    Refresh(RefreshArgs),
    /// 输出版本信息
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecretBackendKind {
    Keyring,
    Env,
    File,
    Memory,
}

impl SecretBackendKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Keyring => "keyring",
            Self::Env => "env",
            Self::File => "file",
            Self::Memory => "memory",
        }
    }
}

#[derive(Args, Debug, Clone)]
pub(crate) struct SecretStoreArgs {
    /// 密钥后端（GUI 默认 keyring）
    #[arg(long, value_enum, default_value_t = SecretBackendKind::Keyring)]
    pub(crate) secrets_backend: SecretBackendKind,
    /// file 后端主密码（也可通过 SUBFORGE_SECRET_KEY 传入）
    #[arg(long)]
    pub(crate) secret_key: Option<String>,
    /// file 后端密钥文件路径，默认 {data_dir}/secrets.enc
    #[arg(long)]
    pub(crate) secrets_file: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct RunArgs {
    /// 无头模式配置文件路径（TOML）
    #[arg(short = 'c', long = "config")]
    pub(crate) config: Option<PathBuf>,
    /// 监听地址，默认仅本机回环
    #[arg(long, default_value = DEFAULT_HOST)]
    pub(crate) host: String,
    /// 监听端口
    #[arg(long, default_value_t = DEFAULT_PORT)]
    pub(crate) port: u16,
    /// GUI 模式，首行输出启动信息 JSON
    #[arg(long, default_value_t = false)]
    pub(crate) gui_mode: bool,
    /// 数据目录，默认当前目录下 .subforge
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) secrets: SecretStoreArgs,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CheckArgs {
    /// 无头模式配置文件路径（TOML）
    #[arg(short = 'c', long = "config")]
    pub(crate) config: Option<PathBuf>,
    /// 数据目录，默认当前目录下 .subforge
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) secrets: SecretStoreArgs,
}

#[derive(Args, Debug)]
pub(crate) struct RefreshArgs {
    /// 来源 ID（预留）
    #[arg(long)]
    pub(crate) source_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GuiBootstrap {
    pub(crate) version: &'static str,
    pub(crate) listen_addr: String,
    pub(crate) port: u16,
    pub(crate) admin_token: String,
    pub(crate) secrets_backend: &'static str,
}
