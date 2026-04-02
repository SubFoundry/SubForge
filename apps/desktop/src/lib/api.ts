import { invoke } from "@tauri-apps/api/core";
import type {
  CoreApiResponse,
  CoreStatus,
  SettingsResponse,
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
  const response = await coreApiCall("GET", "/api/system/settings");
  if (response.status !== 200) {
    throw new Error(`Load settings failed: ${response.status}`);
  }
  return JSON.parse(response.body) as SettingsResponse;
}

export async function updateSystemSettings(
  settings: Record<string, string>,
): Promise<SettingsResponse> {
  const response = await coreApiCall("PUT", "/api/system/settings", { settings });
  if (response.status !== 200) {
    throw new Error(`Update settings failed: ${response.status}`);
  }
  return JSON.parse(response.body) as SettingsResponse;
}
