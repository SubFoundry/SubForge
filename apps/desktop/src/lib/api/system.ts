import type { SettingsResponse, SystemStatusResponse } from "../../types/core";
import { coreApiCall, requestJson } from "./client";

export async function fetchCoreHealth(): Promise<{ status: string; version: string }> {
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
