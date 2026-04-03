import type { LogsResponse, RefreshLog, ScriptRunLog } from "../../types/core";
import { requestJson } from "./client";

export async function fetchRefreshLogs(
  options?: {
    limit?: number;
    offset?: number;
    status?: "running" | "success" | "failed";
    sourceId?: string;
    includeScriptLogs?: boolean;
  },
): Promise<LogsResponse> {
  const limit = options?.limit ?? 20;
  const offset = options?.offset ?? 0;
  const status = options?.status;
  const sourceId = options?.sourceId;
  const includeScriptLogs = options?.includeScriptLogs ?? false;

  const params = new URLSearchParams();
  params.set("limit", String(limit));
  params.set("offset", String(offset));
  if (status) {
    params.set("status", status);
  }
  if (sourceId) {
    params.set("source_id", sourceId);
  }
  if (includeScriptLogs) {
    params.set("include_script_logs", "true");
  }

  const payload = await requestJson<{
    logs: Array<{
      id: string;
      source_id: string;
      source_name?: string | null;
      trigger_type: string;
      status: string;
      started_at?: string | null;
      finished_at?: string | null;
      node_count?: number | null;
      error_code?: string | null;
      error_message?: string | null;
      script_logs?: Array<{
        id: string;
        source_id: string;
        plugin_id: string;
        level: string;
        message: string;
        created_at: string;
      }>;
    }>;
    pagination?: {
      limit: number;
      offset: number;
      total: number;
      has_more: boolean;
    };
  }>("GET", `/api/logs?${params.toString()}`);

  const logs: RefreshLog[] = payload.logs.map((item) => ({
    id: item.id,
    sourceId: item.source_id,
    sourceName: item.source_name ?? null,
    triggerType: item.trigger_type,
    status: item.status,
    startedAt: item.started_at ?? null,
    finishedAt: item.finished_at ?? null,
    nodeCount: item.node_count ?? null,
    errorCode: item.error_code ?? null,
    errorMessage: item.error_message ?? null,
    scriptLogs: mapScriptLogs(item.script_logs),
  }));

  return {
    logs,
    pagination: {
      limit: payload.pagination?.limit ?? limit,
      offset: payload.pagination?.offset ?? offset,
      total: payload.pagination?.total ?? logs.length,
      hasMore: payload.pagination?.has_more ?? false,
    },
  };
}

function mapScriptLogs(
  scriptLogs?: Array<{
    id: string;
    source_id: string;
    plugin_id: string;
    level: string;
    message: string;
    created_at: string;
  }>,
): ScriptRunLog[] {
  return (scriptLogs ?? []).map((log) => ({
    id: log.id,
    sourceId: log.source_id,
    pluginId: log.plugin_id,
    level: log.level,
    message: log.message,
    createdAt: log.created_at,
  }));
}
