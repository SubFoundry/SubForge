import { invoke } from "@tauri-apps/api/core";
import type { CoreApiResponse, CoreStatus } from "../../types/core";

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

export async function desktopAutoCloseGui(): Promise<void> {
  await invoke("desktop_auto_close_gui");
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

export async function requestJson<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const response = await coreApiCall(method, path, body);
  if (response.status < 200 || response.status >= 300) {
    throw new Error(formatApiErrorMessage(method, path, response));
  }
  return JSON.parse(response.body) as T;
}

export function formatApiErrorMessage(
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
    // 忽略非 JSON 错误体，回退到基础错误文本。
  }

  return fallback;
}
