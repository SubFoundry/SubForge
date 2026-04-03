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

export type ConfigSchemaUi = {
  widget?: string;
  placeholder?: string;
  help?: string;
  group?: string;
  order?: number;
};

export type ConfigSchemaProperty = {
  property_type: "string" | "number" | "integer" | "boolean" | string;
  title?: string;
  description?: string;
  default?: unknown;
  enum_values?: unknown[] | null;
  format?: string;
  min_length?: number;
  max_length?: number;
  minimum?: number;
  maximum?: number;
  pattern?: string;
  x_ui?: ConfigSchemaUi;
};

export type ConfigSchema = {
  schema?: string;
  schema_type: "object" | string;
  required: string[];
  properties: Record<string, ConfigSchemaProperty>;
  additional_properties?: boolean;
};

export type PluginSchemaResponse = {
  plugin_id: string;
  name: string;
  plugin_type: string;
  secret_fields: string[];
  schema: ConfigSchema;
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

export type SourceResponse = {
  source: {
    source: SourceRecord;
    config: Record<string, unknown>;
  };
};

export type RefreshSourceResponse = {
  source_id: string;
  node_count: number;
};

export type ProfileRecord = {
  id: string;
  name: string;
  description?: string | null;
  created_at: string;
  updated_at: string;
};

export type ProfileItem = {
  profile: ProfileRecord;
  source_ids: string[];
  export_token?: string | null;
};

export type ProfileListResponse = {
  profiles: ProfileItem[];
};

export type ProfileResponse = {
  profile: ProfileItem;
};

export type RotateProfileExportTokenResponse = {
  profile_id: string;
  token: string;
  previous_token_expires_at: string;
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
  pagination: {
    limit: number;
    offset: number;
    total: number;
    hasMore: boolean;
  };
};
