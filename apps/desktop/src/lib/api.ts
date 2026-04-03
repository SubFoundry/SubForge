import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigSchema,
  CoreApiResponse,
  CoreStatus,
  LogsResponse,
  PluginListResponse,
  PluginRecord,
  PluginSchemaResponse,
  RefreshAllSourcesResult,
  RefreshLog,
  RefreshSourceResponse,
  SettingsResponse,
  SourceListResponse,
  SourceResponse,
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

export async function fetchPlugins(): Promise<PluginListResponse> {
  return requestJson<PluginListResponse>("GET", "/api/plugins");
}

export async function fetchPluginSchema(
  pluginId: string,
): Promise<PluginSchemaResponse> {
  const payload = await requestJson<{
    plugin_id: string;
    name: string;
    plugin_type: string;
    secret_fields?: string[];
    schema: {
      $schema?: string;
      type: string;
      required?: string[];
      properties?: Record<
        string,
        {
          type: string;
          title?: string;
          description?: string;
          default?: unknown;
          enum?: unknown[];
          format?: string;
          minLength?: number;
          maxLength?: number;
          minimum?: number;
          maximum?: number;
          pattern?: string;
          "x-ui"?: {
            widget?: string;
            placeholder?: string;
            help?: string;
            group?: string;
            order?: number;
          };
        }
      >;
      additionalProperties?: boolean;
    };
  }>("GET", `/api/plugins/${encodeURIComponent(pluginId)}/schema`);

  const schema: ConfigSchema = {
    schema: payload.schema.$schema,
    schema_type: payload.schema.type,
    required: payload.schema.required ?? [],
    properties: Object.fromEntries(
      Object.entries(payload.schema.properties ?? {}).map(([fieldName, property]) => [
        fieldName,
        {
          property_type: property.type,
          title: property.title,
          description: property.description,
          default: property.default,
          enum_values: property.enum,
          format: property.format,
          min_length: property.minLength,
          max_length: property.maxLength,
          minimum: property.minimum,
          maximum: property.maximum,
          pattern: property.pattern,
          x_ui: property["x-ui"]
            ? {
                widget: property["x-ui"].widget,
                placeholder: property["x-ui"].placeholder,
                help: property["x-ui"].help,
                group: property["x-ui"].group,
                order: property["x-ui"].order,
              }
            : undefined,
        },
      ]),
    ),
    additional_properties: payload.schema.additionalProperties,
  };

  return {
    plugin_id: payload.plugin_id,
    name: payload.name,
    plugin_type: payload.plugin_type,
    secret_fields: payload.secret_fields ?? [],
    schema,
  };
}

export async function togglePlugin(
  pluginId: string,
  enabled: boolean,
): Promise<PluginRecord> {
  return requestJson<PluginRecord>("PUT", `/api/plugins/${pluginId}/toggle`, { enabled });
}

export async function deletePlugin(pluginId: string): Promise<PluginRecord> {
  return requestJson<PluginRecord>("DELETE", `/api/plugins/${pluginId}`);
}

export async function importPluginZip(file: File): Promise<PluginRecord> {
  if (!file.name.toLowerCase().endsWith(".zip")) {
    throw new Error("仅支持 .zip 插件包");
  }

  const payloadBase64 = await fileToBase64(file);
  const response = await invoke<CoreApiResponse>("core_import_plugin_zip", {
    request: {
      fileName: file.name,
      payloadBase64,
    },
  });

  if (response.status < 200 || response.status >= 300) {
    throw new Error(buildApiErrorMessage("POST", "/api/plugins/import", response));
  }
  return JSON.parse(response.body) as PluginRecord;
}

export async function fetchSources(): Promise<SourceListResponse> {
  return requestJson<SourceListResponse>("GET", "/api/sources");
}

export async function createSource(input: {
  pluginId: string;
  name: string;
  config: Record<string, unknown>;
}): Promise<SourceResponse> {
  return requestJson<SourceResponse>("POST", "/api/sources", {
    plugin_id: input.pluginId,
    name: input.name,
    config: input.config,
  });
}

export async function updateSource(
  sourceId: string,
  input: {
    name?: string;
    config?: Record<string, unknown>;
  },
): Promise<SourceResponse> {
  return requestJson<SourceResponse>(
    "PUT",
    `/api/sources/${encodeURIComponent(sourceId)}`,
    input,
  );
}

export async function deleteSource(sourceId: string): Promise<{ deleted: boolean; id: string }> {
  return requestJson<{ deleted: boolean; id: string }>(
    "DELETE",
    `/api/sources/${encodeURIComponent(sourceId)}`,
  );
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

export async function fetchRefreshLogs(
  options?: {
    limit?: number;
    offset?: number;
    status?: "running" | "success" | "failed";
    sourceId?: string;
  },
): Promise<LogsResponse> {
  const limit = options?.limit ?? 20;
  const offset = options?.offset ?? 0;
  const status = options?.status;
  const sourceId = options?.sourceId;
  const params = new URLSearchParams();
  params.set("limit", String(limit));
  params.set("offset", String(offset));
  if (status) {
    params.set("status", status);
  }
  if (sourceId) {
    params.set("source_id", sourceId);
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

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error("读取插件文件失败"));
        return;
      }
      const marker = "base64,";
      const markerIndex = reader.result.indexOf(marker);
      if (markerIndex < 0) {
        reject(new Error("插件文件编码失败"));
        return;
      }
      resolve(reader.result.slice(markerIndex + marker.length));
    };
    reader.onerror = () => reject(new Error("读取插件文件失败"));
    reader.readAsDataURL(file);
  });
}
