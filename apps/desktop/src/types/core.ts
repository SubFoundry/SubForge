export type CoreStatus = {
  running: boolean;
  baseUrl: string;
  version?: string;
  pid?: number;
};

export type CoreApiResponse = {
  status: number;
  headers: Record<string, string>;
  body: string;
};

export type CoreConnectionPhase =
  | "idle"
  | "booting"
  | "running"
  | "disconnected"
  | "error";

export type CoreEventPayload = {
  event: string;
  message: string;
  sourceId?: string;
  timestamp?: string;
};

export type CoreBridgeEvent = {
  kind: "connected" | "event" | "disconnected" | "error";
  payload?: CoreEventPayload;
  message?: string;
};

export type SettingsResponse = {
  settings: Record<string, string>;
};
