use std::collections::HashMap;

use super::*;

const DEFAULT_LOG_LIMIT: usize = 20;
const MAX_LOG_LIMIT: usize = 200;

pub(crate) async fn list_logs_handler(
    State(state): State<ServerContext>,
    Query(query): Query<LogsQuery>,
) -> ApiResult<LogsResponse> {
    let limit = query.limit.unwrap_or(DEFAULT_LOG_LIMIT);
    if limit == 0 || limit > MAX_LOG_LIMIT {
        return Err(config_error_response("limit 必须在 1..=200 之间"));
    }
    let offset = query.offset.unwrap_or(0);

    let status_filter = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(status) = status_filter
        && !matches!(status, "running" | "success" | "failed")
    {
        return Err(config_error_response(
            "status 仅支持 running/success/failed",
        ));
    }
    let source_id_filter = query
        .source_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let refresh_repository = RefreshJobRepository::new(state.database.as_ref());
    let source_repository = SourceRepository::new(state.database.as_ref());

    let refresh_jobs = if source_id_filter.is_none() && offset == 0 && status_filter.is_some() {
        let status = status_filter.expect("status_filter 已检查为 Some");
        refresh_repository
            .list_recent_by_status(status, limit)
            .map_err(storage_error_to_response)?
    } else if source_id_filter.is_none() && offset == 0 && status_filter.is_none() {
        refresh_repository
            .list_recent(limit)
            .map_err(storage_error_to_response)?
    } else {
        refresh_repository
            .list_recent_filtered(status_filter, source_id_filter, limit, offset)
            .map_err(storage_error_to_response)?
    };
    let total = refresh_repository
        .count_filtered(status_filter, source_id_filter)
        .map_err(storage_error_to_response)?;

    let source_names = source_repository
        .list()
        .map_err(storage_error_to_response)?
        .into_iter()
        .map(|source| (source.id, source.name))
        .collect::<HashMap<_, _>>();

    let logs = refresh_jobs
        .into_iter()
        .map(|job| RefreshLogDto {
            id: job.id,
            source_id: job.source_instance_id.clone(),
            source_name: source_names.get(&job.source_instance_id).cloned(),
            trigger_type: job.trigger_type,
            status: job.status,
            started_at: job.started_at,
            finished_at: job.finished_at,
            node_count: job.node_count,
            error_code: job.error_code,
            error_message: job.error_message,
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(LogsResponse {
            logs,
            pagination: LogsPagination {
                limit,
                offset,
                total,
                has_more: offset.saturating_add(limit) < total,
            },
        }),
    ))
}
