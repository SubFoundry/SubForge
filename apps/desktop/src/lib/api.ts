import { invoke } from "@tauri-apps/api/core";
import type {
  CoreApiResponse,
  CoreStatus,
  RefreshAllSourcesResult,
  RefreshSourceResponse,
  SettingsResponse,
  SourceListResponse,
  SystemStatusResponse,
} from "../types/core";

export async function coreStatus(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_status");
}

export async function coreStart(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_start");
}

export async function coreStop(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_stop");
}

export async function coreEventsStart(): Promise<void> {
  await invoke("core_events_start");
}

export async function coreApiCall(
  method: string,
  path: string,
  body?: unknown,
): Promise<CoreApiResponse> {
  return invoke<CoreApiResponse>("core_api_call", {
    request: {
      method,
      path,
      body: body ?? null,
    },
  });
}

export async function fetchCoreHealth() {
  const response = await coreApiCall("GET", "/health");
  if (response.status !== 200) {
    throw new Error(`Core health request failed: ${response.status}`);
  }
  return JSON.parse(response.body) as { status: string; version: string };
}

export async function fetchSystemSettings(): Promise<SettingsResponse> {
  return requestJson<SettingsResponse>("GET", "/api/system/settings");
}

export async function updateSystemSettings(
  settings: Record<string, string>,
): Promise<SettingsResponse> {
  return requestJson<SettingsResponse>("PUT", "/api/system/settings", { settings });
}

export async function fetchSystemStatus(): Promise<SystemStatusResponse> {
  const payload = await requestJson<{
    active_sources: number;
    total_nodes: number;
    last_refresh_at?: string | null;
  }>("GET", "/api/system/status");

  return {
    activeSources: payload.active_sources,
    totalNodes: payload.total_nodes,
    lastRefreshAt: payload.last_refresh_at ?? null,
  };
}

export async function fetchSources(): Promise<SourceListResponse> {
  return requestJson<SourceListResponse>("GET", "/api/sources");
}

export async function refreshSource(sourceId: string): Promise<RefreshSourceResponse> {
  return requestJson<RefreshSourceResponse>("POST", `/api/sources/${sourceId}/refresh`);
}

export async function refreshAllSources(): Promise<RefreshAllSourcesResult> {
  const list = await fetchSources();
  const succeeded: string[] = [];
  const failed: Array<{ sourceId: string; reason: string }> = [];

  for (const item of list.sources) {
    const sourceId = item.source.id;
    try {
      await refreshSource(sourceId);
      succeeded.push(sourceId);
    } catch (error) {
      failed.push({
        sourceId,
        reason: error instanceof Error ? error.message : "未知错误",
      });
    }
  }

  return {
    total: list.sources.length,
    succeeded,
    failed,
  };
}

async function requestJson<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const response = await coreApiCall(method, path, body);
  if (response.status < 200 || response.status >= 300) {
    throw new Error(buildApiErrorMessage(method, path, response));
  }
  return JSON.parse(response.body) as T;
}

function buildApiErrorMessage(
  method: string,
  path: string,
  response: CoreApiResponse,
): string {
  const fallback = `${method} ${path} failed: ${response.status}`;

  try {
    const parsed = JSON.parse(response.body) as {
      message?: string;
      code?: string;
    };
    if (parsed.message && parsed.code) {
      return `${fallback} (${parsed.code}: ${parsed.message})`;
    }
    if (parsed.message) {
      return `${fallback} (${parsed.message})`;
    }
  } catch {
    // ignore non-json error body
  }

  return fallback;
}
