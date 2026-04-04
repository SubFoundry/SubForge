import { Skeleton } from "../../components/skeleton";
import type { ConfigSchemaProperty, PluginRecord, PluginSchemaResponse } from "../../types/core";
import type { SourceFormMode } from "./utils";
import { SourceField } from "./source-field";

type SourceFormCardProps = {
  mode: SourceFormMode;
  pluginId: string;
  sourceName: string;
  enabledPlugins: PluginRecord[];
  schemaPayload?: PluginSchemaResponse;
  schemaLoading: boolean;
  fields: Array<{ key: string; property: ConfigSchemaProperty }>;
  formConfig: Record<string, unknown>;
  keptSecretFields: string[];
  submitDisabled: boolean;
  submitting: boolean;
  onPluginIdChange: (pluginId: string) => void;
  onSourceNameChange: (value: string) => void;
  onConfigChange: (fieldKey: string, nextValue: unknown) => void;
  onKeepSecretChange: (fieldKey: string, keep: boolean) => void;
  onSubmit: () => void;
  onCancelEdit: () => void;
};

export function SourceFormCard({
  mode,
  pluginId,
  sourceName,
  enabledPlugins,
  schemaPayload,
  schemaLoading,
  fields,
  formConfig,
  keptSecretFields,
  submitDisabled,
  submitting,
  onPluginIdChange,
  onSourceNameChange,
  onConfigChange,
  onKeepSecretChange,
  onSubmit,
  onCancelEdit,
}: SourceFormCardProps) {
  return (
    <article className="ui-card">
      <div className="ui-card-header">
        <div>
          <h3 className="ui-card-title">{mode === "create" ? "创建来源" : "编辑来源"}</h3>
          <p className="ui-card-desc">按插件 Schema 自动渲染字段，敏感字段走密钥占位保留。</p>
        </div>
      </div>

      <div className="ui-card-body space-y-4">
        <div className="grid gap-3 md:grid-cols-2">
          <label className="text-xs text-[var(--muted-text)]">
            <span className="text-[var(--app-text)]">插件</span>
            <select
              className="ui-select ui-focus mt-1 disabled:opacity-70"
              value={pluginId}
              disabled={mode === "edit"}
              onChange={(event) => onPluginIdChange(event.currentTarget.value)}
            >
              {enabledPlugins.length === 0 && <option value="">暂无可用插件</option>}
              {enabledPlugins.map((plugin) => (
                <option key={plugin.id} value={plugin.plugin_id}>
                  {plugin.name} ({plugin.plugin_id})
                </option>
              ))}
            </select>
          </label>

          <label className="text-xs text-[var(--muted-text)]">
            <span className="text-[var(--app-text)]">来源名称</span>
            <input
              className="ui-input ui-focus mt-1"
              value={sourceName}
              onChange={(event) => onSourceNameChange(event.currentTarget.value)}
              placeholder="例如：主订阅 / 备用订阅"
            />
          </label>
        </div>

        {schemaLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
          </div>
        ) : !schemaPayload ? (
          <p className="text-sm text-[var(--muted-text)]">
            请先选择插件，随后按 Schema 渲染配置表单。
          </p>
        ) : (
          <div className="grid gap-3 md:grid-cols-2">
            {fields.map(({ key, property }) => (
              <SourceField
                key={key}
                fieldKey={key}
                property={property}
                value={formConfig[key]}
                required={schemaPayload.schema.required.includes(key)}
                secret={schemaPayload.secret_fields.includes(key)}
                keepSecret={keptSecretFields.includes(key)}
                onChange={(nextValue) => onConfigChange(key, nextValue)}
                onKeepSecretChange={(keep) => onKeepSecretChange(key, keep)}
              />
            ))}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="ui-btn ui-btn-primary ui-focus"
            disabled={submitDisabled}
            onClick={onSubmit}
          >
            {submitting ? "提交中..." : mode === "create" ? "创建来源" : "保存修改"}
          </button>
          {mode === "edit" && (
            <button
              type="button"
              className="ui-btn ui-btn-secondary ui-focus"
              onClick={onCancelEdit}
            >
              取消编辑
            </button>
          )}
        </div>
      </div>
    </article>
  );
}
