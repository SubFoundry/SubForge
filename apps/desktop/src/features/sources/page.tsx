import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  fetchPluginSchema,
  fetchPlugins,
  fetchSources,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { ConfigSchemaProperty, SourceListResponse } from "../../types/core";
import { SourceFormCard } from "./source-form-card";
import { SourceListCard } from "./source-list-card";
import { useSourceActions } from "./use-source-actions";
import {
  buildInitialFormConfig,
  normalizeFormConfigForSubmit,
  type SourceFormMode,
} from "./utils";

type SourceListItem = SourceListResponse["sources"][number];

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
    () => (pluginsQuery.data?.plugins ?? []).filter((plugin) => plugin.status !== "disabled"),
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
    return sourcesQuery.data?.sources.find((item) => item.source.id === editingSourceId) ?? null;
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
  }, [editingSourceId, formMode, pluginSchemaQuery.data, selectedSource]);

  const {
    activeSourceId,
    setActiveSourceId,
    createMutation,
    updateMutation,
    refreshMutation,
    deleteMutation,
  } = useSourceActions({
    queryClient,
    addToast,
    onCreateSuccess: beginCreate,
  });

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

  const handlePluginIdChange = (pluginId: string) => {
    setFormPluginId(pluginId);
    setFormConfig({});
    setKeptSecretFields([]);
    initializedFormKeyRef.current = "";
  };

  const handleConfigChange = (fieldKey: string, nextValue: unknown) => {
    setFormConfig((current) => ({
      ...current,
      [fieldKey]: nextValue,
    }));
  };

  const handleKeepSecretChange = (fieldKey: string, keep: boolean) => {
    if (formMode !== "edit") {
      return;
    }
    setKeptSecretFields((current) => {
      const withoutCurrent = current.filter((item) => item !== fieldKey);
      return keep ? [...withoutCurrent, fieldKey] : withoutCurrent;
    });
  };

  const handleDelete = (item: SourceListItem) => {
    const source = item.source;
    const confirmed = window.confirm(
      `确认删除来源 "${source.name}" 吗？该操作会移除相关配置与密钥引用。`,
    );
    if (!confirmed) {
      return;
    }
    setActiveSourceId(source.id);
    deleteMutation.mutate(source.id);
  };

  const handleRefresh = (sourceId: string) => {
    setActiveSourceId(sourceId);
    refreshMutation.mutate(sourceId);
  };

  const beginEdit = (item: SourceListItem) => {
    setFormMode("edit");
    setEditingSourceId(item.source.id);
    setFormPluginId(item.source.plugin_id);
    setFormName(item.source.name);
    setFormConfig({});
    setKeptSecretFields([]);
    initializedFormKeyRef.current = "";
  };

  function beginCreate() {
    setFormMode("create");
    setEditingSourceId(null);
    setFormName("");
    setFormConfig({});
    setKeptSecretFields([]);
    initializedFormKeyRef.current = "";
    if (enabledPlugins.length > 0) {
      setFormPluginId(enabledPlugins[0].plugin_id);
    }
  }

  return (
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Sources</h2>
          <p className="ui-page-desc">动态来源配置区与来源列表区分离，降低误操作概率。</p>
        </div>
        <button type="button" className="ui-btn ui-btn-secondary ui-focus" onClick={beginCreate}>
          新建来源
        </button>
      </header>

      <SourceFormCard
        mode={formMode}
        pluginId={formPluginId}
        sourceName={formName}
        enabledPlugins={enabledPlugins}
        schemaPayload={pluginSchemaQuery.data}
        schemaLoading={pluginSchemaQuery.isLoading}
        fields={fields}
        formConfig={formConfig}
        keptSecretFields={keptSecretFields}
        submitDisabled={submitDisabled}
        submitting={createMutation.isPending || updateMutation.isPending}
        onPluginIdChange={handlePluginIdChange}
        onSourceNameChange={setFormName}
        onConfigChange={handleConfigChange}
        onKeepSecretChange={handleKeepSecretChange}
        onSubmit={handleSubmit}
        onCancelEdit={beginCreate}
      />

      <SourceListCard
        loading={sourcesQuery.isLoading}
        sources={sourcesQuery.data?.sources ?? []}
        activeSourceId={activeSourceId}
        onRefresh={handleRefresh}
        onEdit={beginEdit}
        onDelete={handleDelete}
      />
    </section>
  );
}
