import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigSchema,
  CoreApiResponse,
  PluginListResponse,
  PluginRecord,
  PluginSchemaResponse,
} from "../../types/core";
import { formatApiErrorMessage, requestJson } from "./client";

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
    throw new Error(formatApiErrorMessage("POST", "/api/plugins/import", response));
  }
  return JSON.parse(response.body) as PluginRecord;
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
