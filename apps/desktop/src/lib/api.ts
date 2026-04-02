import { invoke } from "@tauri-apps/api/core";
import type { CoreApiResponse, CoreStatus } from "../types/core";

export async function coreStatus(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_status");
}

export async function coreStart(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_start");
}

export async function coreStop(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_stop");
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