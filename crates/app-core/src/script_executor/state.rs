use app_common::SourceInstance;
use app_storage::{Database, SourceRepository};
use serde_json::Value;

use crate::script_executor::StateUpdate;
use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult};

pub(super) fn apply_state_update(state: &mut Option<Value>, update: StateUpdate) {
    match update {
        StateUpdate::Keep => {}
        StateUpdate::Replace(next) => *state = next,
    }
}

pub(super) fn persist_state_if_changed(
    db: &Database,
    source: &mut SourceInstance,
    state: &Option<Value>,
    update: &StateUpdate,
) -> CoreResult<()> {
    if matches!(update, StateUpdate::Keep) {
        return Ok(());
    }

    source.state_json = match state {
        Some(value) => Some(serde_json::to_string(value).map_err(|error| {
            CoreError::ConfigInvalid(format!("脚本 state 序列化失败：{error}"))
        })?),
        None => None,
    };
    source.updated_at = now_rfc3339()?;
    let repository = SourceRepository::new(db);
    repository.update(source)?;
    Ok(())
}

pub(super) fn parse_persisted_state(raw: Option<&str>) -> CoreResult<Option<Value>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<Value>(raw)
        .map_err(|error| CoreError::ConfigInvalid(format!("state_json 反序列化失败：{error}")))?;
    if parsed.is_null() {
        return Ok(None);
    }
    if !parsed.is_object() {
        return Err(CoreError::ConfigInvalid(
            "state_json 必须是 JSON 对象".to_string(),
        ));
    }
    Ok(Some(parsed))
}
