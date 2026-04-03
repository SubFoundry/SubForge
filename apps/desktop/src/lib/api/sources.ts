import type {
  RefreshAllSourcesResult,
  RefreshSourceResponse,
  SourceListResponse,
  SourceResponse,
} from "../../types/core";
import { requestJson } from "./client";

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

export async function deleteSource(
  sourceId: string,
): Promise<{ deleted: boolean; id: string }> {
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
