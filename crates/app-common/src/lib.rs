//! app-common：公共模型与错误定义。

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ErrorResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub spec_version: String,
    pub plugin_type: String,
    pub status: String,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceInstance {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub status: String,
    pub state_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileSource {
    pub profile_id: String,
    pub source_instance_id: String,
    pub priority: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSetting {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_error_fields_are_serializable() {
        let err = AppError::new("E_TEST", "测试错误", false);
        let json = serde_json::to_string(&err).expect("序列化失败");
        assert!(json.contains("\"code\":\"E_TEST\""));
        assert!(json.contains("\"message\":\"测试错误\""));
        assert!(json.contains("\"retryable\":false"));
    }

    #[test]
    fn domain_models_are_serializable() {
        let plugin = Plugin {
            id: "plugin-row-1".to_string(),
            plugin_id: "vendor.example.static".to_string(),
            name: "Example Plugin".to_string(),
            version: "1.0.0".to_string(),
            spec_version: "1.0".to_string(),
            plugin_type: "static".to_string(),
            status: "enabled".to_string(),
            installed_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let source = SourceInstance {
            id: "source-1".to_string(),
            plugin_id: plugin.plugin_id.clone(),
            name: "Source A".to_string(),
            status: "healthy".to_string(),
            state_json: Some("{\"cursor\":1}".to_string()),
            created_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let profile = Profile {
            id: "profile-1".to_string(),
            name: "Default".to_string(),
            description: Some("默认聚合配置".to_string()),
            created_at: "2026-04-02T00:00:00Z".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };
        let profile_source = ProfileSource {
            profile_id: profile.id.clone(),
            source_instance_id: source.id.clone(),
            priority: 10,
        };
        let setting = AppSetting {
            key: "ui.theme".to_string(),
            value: "dark".to_string(),
            updated_at: "2026-04-02T00:00:00Z".to_string(),
        };

        assert!(
            serde_json::from_str::<Plugin>(
                &serde_json::to_string(&plugin).expect("plugin 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<SourceInstance>(
                &serde_json::to_string(&source).expect("source 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<Profile>(
                &serde_json::to_string(&profile).expect("profile 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<ProfileSource>(
                &serde_json::to_string(&profile_source).expect("profile_source 序列化失败")
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<AppSetting>(
                &serde_json::to_string(&setting).expect("setting 序列化失败")
            )
            .is_ok()
        );
    }
}
