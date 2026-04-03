import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Skeleton } from "../../components/skeleton";
import {
  createSource,
  deleteSource,
  fetchPluginSchema,
  fetchPlugins,
  fetchSources,
  refreshSource,
  updateSource,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type {
  ConfigSchema,
  ConfigSchemaProperty,
  SourceRecord,
} from "../../types/core";

const SECRET_PLACEHOLDER = "••••••";

type SourceFormMode = "create" | "edit";

export default function SourcesPage() {
  const queryClient = useQueryClient();
  const addToast = useCoreUiStore((state) => state.addToast);
  const phase = useCoreUiStore((state) => state.phase);

  const [formMode, setFormMode] = useState<SourceFormMode>("create");
  const [editingSourceId, setEditingSourceId] = useState<string | null>(null);
  const [formPluginId, setFormPluginId] = useState("");
  const [formName, setFormName] = useState("");
  const [formConfig, setFormConfig] = useState<Record<string, unknown>>({});
  const [keptSecretFields, setKeptSecretFields] = useState<string[]>([]);
  const [activeSourceId, setActiveSourceId] = useState<string | null>(null);
  const initializedFormKeyRef = useRef<string>("");

  const pluginsQuery = useQuery({
    queryKey: ["plugins"],
    queryFn: fetchPlugins,
    enabled: phase === "running",
    refetchInterval: 30_000,
  });

  const sourcesQuery = useQuery({
    queryKey: ["sources"],
    queryFn: fetchSources,
    enabled: phase === "running",
    refetchInterval: 15_000,
  });

  const enabledPlugins = useMemo(
    () =>
      (pluginsQuery.data?.plugins ?? []).filter(
        (plugin) => plugin.status !== "disabled",
      ),
    [pluginsQuery.data?.plugins],
  );

  useEffect(() => {
    if (formPluginId || enabledPlugins.length === 0 || formMode !== "create") {
      return;
    }
    setFormPluginId(enabledPlugins[0].plugin_id);
  }, [enabledPlugins, formMode, formPluginId]);

  const selectedSource = useMemo(() => {
    if (!editingSourceId) {
      return null;
    }
    return (
      sourcesQuery.data?.sources.find((item) => item.source.id === editingSourceId) ?? null
    );
  }, [editingSourceId, sourcesQuery.data?.sources]);

  const pluginSchemaQuery = useQuery({
    queryKey: ["source-plugin-schema", formPluginId],
    queryFn: () => fetchPluginSchema(formPluginId),
    enabled: phase === "running" && formPluginId.length > 0,
    staleTime: 60_000,
  });

  useEffect(() => {
    const schemaPayload = pluginSchemaQuery.data;
    if (!schemaPayload) {
      return;
    }

    const initializeKey = `${formMode}:${editingSourceId ?? "new"}:${schemaPayload.plugin_id}`;
    if (initializedFormKeyRef.current === initializeKey) {
      return;
    }

    const existingConfig = formMode === "edit" ? selectedSource?.config : undefined;
    const defaults = buildInitialFormConfig(
      schemaPayload.schema,
      schemaPayload.secret_fields,
      existingConfig,
    );

    setFormConfig(defaults.values);
    setKeptSecretFields(defaults.keptSecretFields);
    if (formMode === "edit" && selectedSource) {
      setFormName(selectedSource.source.name);
    }
    initializedFormKeyRef.current = initializeKey;
  }, [
    editingSourceId,
    formMode,
    pluginSchemaQuery.data,
    selectedSource,
    selectedSource?.config,
  ]);

  const createMutation = useMutation({
    mutationFn: createSource,
    onSuccess: (payload) => {
      addToast({
        title: "来源创建成功",
        description: payload.source.source.name,
        variant: "default",
      });
      setFormMode("create");
      setEditingSourceId(null);
      setFormName("");
      setFormConfig({});
      setKeptSecretFields([]);
      initializedFormKeyRef.current = "";
      if (enabledPlugins.length > 0) {
        setFormPluginId(enabledPlugins[0].plugin_id);
      }
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源创建失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (input: {
      sourceId: string;
      name: string;
      config: Record<string, unknown>;
    }) => updateSource(input.sourceId, { name: input.name, config: input.config }),
    onSuccess: (payload) => {
      addToast({
        title: "来源更新成功",
        description: payload.source.source.name,
        variant: "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
    },
    onError: (error) => {
      addToast({
        title: "来源更新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveSourceId(null);
    },
  });

  const refreshMutation = useMutation({
    mutationFn: (sourceId: string) => refreshSource(sourceId),
    onSuccess: (payload) => {
      addToast({
        title: "来源刷新成功",
        description: `${payload.source_id} 返回 ${payload.node_count} 个节点`,
        variant: "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["runs", "logs"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源刷新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveSourceId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (sourceId: string) => deleteSource(sourceId),
    onSuccess: () => {
      addToast({
        title: "来源已删除",
        description: "来源记录及其关联缓存已清理。",
        variant: "warning",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["runs", "sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源删除失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveSourceId(null);
    },
  });

  const submitDisabled =
    createMutation.isPending ||
    updateMutation.isPending ||
    !pluginSchemaQuery.data ||
    !formPluginId ||
    !formName.trim();

  const handleSubmit = () => {
    const schemaPayload = pluginSchemaQuery.data;
    if (!schemaPayload) {
      return;
    }

    const normalizedConfig = normalizeFormConfigForSubmit(
      schemaPayload.schema,
      schemaPayload.secret_fields,
      formConfig,
      keptSecretFields,
    );
    const trimmedName = formName.trim();
    if (formMode === "create") {
      createMutation.mutate({
        pluginId: formPluginId,
        name: trimmedName,
        config: normalizedConfig,
      });
      return;
    }

    if (!editingSourceId) {
      return;
    }
    setActiveSourceId(editingSourceId);
    updateMutation.mutate({
      sourceId: editingSourceId,
      name: trimmedName,
      config: normalizedConfig,
    });
  };

  const beginCreate = () => {
    setFormMode("create");
    setEditingSourceId(null);
    setFormName("");
    setFormConfig({});
    setKeptSecretFields([]);
    initializedFormKeyRef.current = "";
    if (enabledPlugins.length > 0) {
      setFormPluginId(enabledPlugins[0].plugin_id);
    }
  };

  const beginEdit = (source: { source: SourceRecord; config: Record<string, unknown> }) => {
    setFormMode("edit");
    setEditingSourceId(source.source.id);
    setFormPluginId(source.source.plugin_id);
    setFormName(source.source.name);
    setFormConfig({});
    setKeptSecretFields([]);
    initializedFormKeyRef.current = "";
  };

  const handleDelete = (source: SourceRecord) => {
    const confirmed = window.confirm(
      `确认删除来源 "${source.name}" 吗？该操作会移除相关配置与密钥引用。`,
    );
    if (!confirmed) {
      return;
    }
    setActiveSourceId(source.id);
    deleteMutation.mutate(source.id);
  };

  const fields = useMemo(() => {
    if (!pluginSchemaQuery.data) {
      return [] as Array<{ key: string; property: ConfigSchemaProperty }>;
    }
    return Object.entries(pluginSchemaQuery.data.schema.properties)
      .map(([key, property]) => ({ key, property }))
      .sort((left, right) => {
        const leftOrder = left.property.x_ui?.order ?? Number.MAX_SAFE_INTEGER;
        const rightOrder = right.property.x_ui?.order ?? Number.MAX_SAFE_INTEGER;
        if (leftOrder !== rightOrder) {
          return leftOrder - rightOrder;
        }
        return left.key.localeCompare(right.key, "zh-CN");
      });
  }, [pluginSchemaQuery.data]);

  return (
    <section className="space-y-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-2xl font-semibold">Sources</h2>
          <p className="mt-1 text-sm text-[var(--muted-text)]">
            来源实例管理与动态表单配置（按插件 schema 渲染）。
          </p>
        </div>
        <button
          type="button"
          className="rounded-lg border border-[var(--panel-border)] px-3 py-2 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
          onClick={beginCreate}
        >
          新建来源
        </button>
      </header>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">
          {formMode === "create" ? "创建来源" : "编辑来源"}
        </h3>

        <div className="mt-3 grid gap-3 md:grid-cols-2">
          <label className="text-xs text-[var(--muted-text)]">
            插件
            <select
              className="mt-1 w-full rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)] disabled:opacity-70"
              value={formPluginId}
              disabled={formMode === "edit"}
              onChange={(event) => {
                setFormPluginId(event.currentTarget.value);
                setFormConfig({});
                setKeptSecretFields([]);
                initializedFormKeyRef.current = "";
              }}
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
            来源名称
            <input
              className="mt-1 w-full rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]"
              value={formName}
              onChange={(event) => setFormName(event.currentTarget.value)}
              placeholder="例如：主订阅 / 备用订阅"
            />
          </label>
        </div>

        <div className="mt-4">
          {pluginSchemaQuery.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : !pluginSchemaQuery.data ? (
            <p className="text-sm text-[var(--muted-text)]">
              请先选择插件，随后会按 schema 渲染配置表单。
            </p>
          ) : (
            <div className="grid gap-3 md:grid-cols-2">
              {fields.map(({ key, property }) => (
                <SourceField
                  key={key}
                  fieldKey={key}
                  property={property}
                  value={formConfig[key]}
                  required={pluginSchemaQuery.data.schema.required.includes(key)}
                  secret={pluginSchemaQuery.data.secret_fields.includes(key)}
                  keepSecret={keptSecretFields.includes(key)}
                  onChange={(nextValue) => {
                    setFormConfig((current) => ({
                      ...current,
                      [key]: nextValue,
                    }));
                  }}
                  onKeepSecretChange={(keep) => {
                    if (formMode !== "edit") {
                      return;
                    }
                    setKeptSecretFields((current) => {
                      const withoutCurrent = current.filter((item) => item !== key);
                      return keep ? [...withoutCurrent, key] : withoutCurrent;
                    });
                  }}
                />
              ))}
            </div>
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="rounded-lg bg-[var(--accent-soft)] px-3 py-2 text-xs font-semibold text-[var(--accent-strong)] transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
            disabled={submitDisabled}
            onClick={handleSubmit}
          >
            {createMutation.isPending || updateMutation.isPending
              ? "提交中..."
              : formMode === "create"
                ? "创建来源"
                : "保存修改"}
          </button>
          {formMode === "edit" && (
            <button
              type="button"
              className="rounded-lg border border-[var(--panel-border)] px-3 py-2 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
              onClick={beginCreate}
            >
              取消编辑
            </button>
          )}
        </div>
      </article>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">来源列表</h3>
        {sourcesQuery.isLoading ? (
          <div className="mt-3 space-y-2">
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </div>
        ) : (sourcesQuery.data?.sources ?? []).length === 0 ? (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无来源，请先创建。</p>
        ) : (
          <div className="mt-3 space-y-2">
            {(sourcesQuery.data?.sources ?? []).map((item) => {
              const source = item.source;
              const busy = activeSourceId === source.id;
              return (
                <article
                  key={source.id}
                  className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-3 py-3 text-sm"
                >
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="font-medium text-[var(--app-text)]">{source.name}</p>
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        {source.plugin_id} | 创建：{formatTimestamp(source.created_at)} | 更新：
                        {formatTimestamp(source.updated_at)}
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className={`rounded-full px-2 py-1 text-xs ${statusClass(source.status)}`}>
                        {source.status}
                      </span>
                      <button
                        type="button"
                        className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)] disabled:cursor-not-allowed disabled:opacity-60"
                        disabled={busy}
                        onClick={() => {
                          setActiveSourceId(source.id);
                          refreshMutation.mutate(source.id);
                        }}
                      >
                        刷新
                      </button>
                      <button
                        type="button"
                        className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
                        onClick={() => beginEdit(item)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="rounded-md border border-rose-400/35 px-2 py-1 text-xs text-rose-300 transition hover:bg-rose-500/15 disabled:cursor-not-allowed disabled:opacity-60"
                        disabled={busy}
                        onClick={() => handleDelete(source)}
                      >
                        删除
                      </button>
                    </div>
                  </div>
                  <pre className="mt-3 overflow-x-auto rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-xs text-[var(--muted-text)]">
                    {JSON.stringify(item.config, null, 2)}
                  </pre>
                </article>
              );
            })}
          </div>
        )}
      </article>
    </section>
  );
}

function SourceField({
  fieldKey,
  property,
  value,
  required,
  secret,
  keepSecret,
  onChange,
  onKeepSecretChange,
}: {
  fieldKey: string;
  property: ConfigSchemaProperty;
  value: unknown;
  required: boolean;
  secret: boolean;
  keepSecret: boolean;
  onChange: (value: unknown) => void;
  onKeepSecretChange: (keep: boolean) => void;
}) {
  const label = property.title ?? fieldKey;
  const enumValues = property.enum_values ?? [];
  const baseClass =
    "mt-1 w-full rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]";

  if (enumValues.length > 0) {
    const selectedValue = value === undefined || value === null ? "" : JSON.stringify(value);
    return (
      <label className="text-xs text-[var(--muted-text)]">
        {label}
        {required ? " *" : ""}
        <select
          className={baseClass}
          value={selectedValue}
          onChange={(event) => {
            if (!event.currentTarget.value) {
              onChange("");
              return;
            }
            onChange(JSON.parse(event.currentTarget.value));
          }}
        >
          <option value="">请选择</option>
          {enumValues.map((item) => {
            const optionValue = JSON.stringify(item);
            return (
              <option key={optionValue} value={optionValue}>
                {String(item)}
              </option>
            );
          })}
        </select>
        <FieldHint property={property} />
      </label>
    );
  }

  if (property.property_type === "boolean") {
    return (
      <label className="flex items-center gap-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]">
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span>
          {label}
          {required ? " *" : ""}
        </span>
      </label>
    );
  }

  if (property.property_type === "number" || property.property_type === "integer") {
    return (
      <label className="text-xs text-[var(--muted-text)]">
        {label}
        {required ? " *" : ""}
        <input
          className={baseClass}
          type="number"
          step={property.property_type === "integer" ? "1" : "any"}
          min={property.minimum}
          max={property.maximum}
          placeholder={property.x_ui?.placeholder}
          value={value === undefined || value === null ? "" : String(value)}
          onChange={(event) => {
            const raw = event.currentTarget.value;
            onChange(raw === "" ? "" : Number(raw));
          }}
        />
        <FieldHint property={property} />
      </label>
    );
  }

  const isPassword = secret || property.format === "password";
  return (
    <label className="text-xs text-[var(--muted-text)]">
      {label}
      {required ? " *" : ""}
      <input
        className={baseClass}
        type={isPassword ? "password" : "text"}
        minLength={property.min_length}
        maxLength={property.max_length}
        pattern={property.pattern}
        placeholder={
          isPassword && keepSecret
            ? "留空保持现有密钥"
            : property.x_ui?.placeholder ?? property.description
        }
        value={typeof value === "string" ? value : value === undefined ? "" : String(value)}
        onChange={(event) => {
          const next = event.currentTarget.value;
          onChange(next);
          if (isPassword) {
            onKeepSecretChange(next.length === 0);
          }
        }}
      />
      <FieldHint property={property} />
    </label>
  );
}

function FieldHint({ property }: { property: ConfigSchemaProperty }) {
  if (!property.x_ui?.help && !property.description) {
    return null;
  }
  return (
    <p className="mt-1 text-[11px] text-[var(--muted-text)]">
      {property.x_ui?.help ?? property.description}
    </p>
  );
}

function normalizeFormConfigForSubmit(
  schema: ConfigSchema,
  secretFields: string[],
  formConfig: Record<string, unknown>,
  keptSecretFields: string[],
): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  const requiredSet = new Set(schema.required);
  const keptSecretSet = new Set(keptSecretFields);
  const secretFieldSet = new Set(secretFields);

  for (const [fieldKey, property] of Object.entries(schema.properties)) {
    const rawValue = formConfig[fieldKey];
    if (secretFieldSet.has(fieldKey) && keptSecretSet.has(fieldKey) && !rawValue) {
      result[fieldKey] = SECRET_PLACEHOLDER;
      continue;
    }

    if (rawValue === undefined || rawValue === null || rawValue === "") {
      if (property.default !== undefined) {
        result[fieldKey] = property.default;
      } else if (requiredSet.has(fieldKey) && property.property_type === "boolean") {
        result[fieldKey] = false;
      } else if (requiredSet.has(fieldKey)) {
        result[fieldKey] = "";
      } else {
        continue;
      }
      continue;
    }

    if (property.property_type === "integer") {
      result[fieldKey] = Math.trunc(Number(rawValue));
      continue;
    }

    if (property.property_type === "number") {
      result[fieldKey] = Number(rawValue);
      continue;
    }

    if (property.property_type === "boolean") {
      result[fieldKey] = Boolean(rawValue);
      continue;
    }

    result[fieldKey] = rawValue;
  }

  return result;
}

function buildInitialFormConfig(
  schema: ConfigSchema,
  secretFields: string[],
  existingConfig?: Record<string, unknown>,
): {
  values: Record<string, unknown>;
  keptSecretFields: string[];
} {
  const values: Record<string, unknown> = {};
  const keptSecretFields: string[] = [];
  const secretFieldSet = new Set(secretFields);

  for (const [fieldKey, property] of Object.entries(schema.properties)) {
    const currentValue = existingConfig?.[fieldKey];
    if (secretFieldSet.has(fieldKey) && currentValue === SECRET_PLACEHOLDER) {
      values[fieldKey] = "";
      keptSecretFields.push(fieldKey);
      continue;
    }

    if (currentValue !== undefined) {
      values[fieldKey] = currentValue;
      continue;
    }

    if (property.default !== undefined) {
      values[fieldKey] = property.default;
      continue;
    }

    if (property.property_type === "boolean") {
      values[fieldKey] = false;
      continue;
    }

    values[fieldKey] = "";
  }

  return { values, keptSecretFields };
}

function statusClass(status: string): string {
  if (status === "healthy" || status === "enabled" || status === "success") {
    return "bg-emerald-500/20 text-emerald-300";
  }
  if (status === "degraded" || status === "running") {
    return "bg-amber-500/20 text-amber-300";
  }
  return "bg-rose-500/20 text-rose-300";
}

function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString("zh-CN", { hour12: false });
}
