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

export type PluginRecord = {
  id: string;
  plugin_id: string;
  name: string;
  version: string;
  spec_version: string;
  plugin_type: string;
  status: "enabled" | "disabled" | string;
  installed_at: string;
  updated_at: string;
};

export type PluginListResponse = {
  plugins: PluginRecord[];
};

export type SourceRecord = {
  id: string;
  plugin_id: string;
  name: string;
  status: string;
  state_json?: string | null;
  created_at: string;
  updated_at: string;
};

export type SourceListResponse = {
  sources: Array<{
    source: SourceRecord;
    config: Record<string, unknown>;
  }>;
};

export type RefreshSourceResponse = {
  source_id: string;
  node_count: number;
};

export type SystemStatusResponse = {
  activeSources: number;
  totalNodes: number;
  lastRefreshAt: string | null;
};

export type RefreshAllSourcesResult = {
  total: number;
  succeeded: string[];
  failed: Array<{ sourceId: string; reason: string }>;
};

export type RefreshLog = {
  id: string;
  sourceId: string;
  sourceName: string | null;
  triggerType: string;
  status: "running" | "success" | "failed" | string;
  startedAt: string | null;
  finishedAt: string | null;
  nodeCount: number | null;
  errorCode: string | null;
  errorMessage: string | null;
};

export type LogsResponse = {
  logs: RefreshLog[];
};
