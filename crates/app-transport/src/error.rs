use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("不支持的 network_profile：{0}")]
    UnsupportedProfile(String),
    #[error("HTTP 客户端初始化失败：{0}")]
    ClientBuild(#[from] reqwest::Error),
}

impl TransportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedProfile(_) => "E_CONFIG_INVALID",
            Self::ClientBuild(_) => "E_INTERNAL",
        }
    }
}

pub type TransportResult<T> = Result<T, TransportError>;
