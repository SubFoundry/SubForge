import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import {
  deletePlugin,
  fetchPlugins,
  importPluginZip,
  togglePlugin,
} from "../../lib/api";
import { queryKeys } from "../../lib/query-keys";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { PluginRecord } from "../../types/core";
import { PluginImportCard } from "./plugin-import-card";
import { PluginListCard } from "./plugin-list-card";

export default function PluginsPage() {
  const queryClient = useQueryClient();
  const addToast = useCoreUiStore((state) => state.addToast);
  const phase = useCoreUiStore((state) => state.phase);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [expandedPluginId, setExpandedPluginId] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [activePluginId, setActivePluginId] = useState<string | null>(null);

  const pluginsQuery = useQuery({
    queryKey: queryKeys.plugins.all,
    queryFn: fetchPlugins,
    refetchInterval: eventStreamActive ? 45_000 : 20_000,
    enabled: phase === "running",
  });

  const importMutation = useMutation({
    mutationFn: importPluginZip,
    onSuccess: (plugin) => {
      setUploadError(null);
      addToast({
        title: "插件导入成功",
        description: `${plugin.name} (${plugin.plugin_id})`,
        variant: "default",
      });
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all });
      }
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : "导入插件失败";
      setUploadError(message);
      addToast({
        title: "插件导入失败",
        description: message,
        variant: "error",
      });
    },
    onSettled: () => {
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    },
  });

  const toggleMutation = useMutation({
    mutationFn: (variables: { pluginId: string; enabled: boolean }) =>
      togglePlugin(variables.pluginId, variables.enabled),
    onSuccess: (plugin) => {
      addToast({
        title: plugin.status === "enabled" ? "插件已启用" : "插件已禁用",
        description: `${plugin.name} (${plugin.plugin_id})`,
        variant: "default",
      });
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all });
      }
    },
    onError: (error) => {
      addToast({
        title: "更新插件状态失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActivePluginId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (pluginId: string) => deletePlugin(pluginId),
    onSuccess: (plugin) => {
      addToast({
        title: "插件已删除",
        description: `${plugin.name} (${plugin.plugin_id})`,
        variant: "warning",
      });
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all });
      }
    },
    onError: (error) => {
      addToast({
        title: "删除插件失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActivePluginId(null);
    },
  });

  const plugins = pluginsQuery.data?.plugins ?? [];
  const isUploading = importMutation.isPending;

  const handleImportFile = (file: File | null) => {
    if (!file) {
      return;
    }
    importMutation.mutate(file);
  };

  const handleToggle = (plugin: PluginRecord) => {
    setActivePluginId(plugin.id);
    toggleMutation.mutate({
      pluginId: plugin.id,
      enabled: plugin.status !== "enabled",
    });
  };

  const handleDelete = (plugin: PluginRecord) => {
    const confirmed = window.confirm(
      `确认删除插件 "${plugin.name}" (${plugin.plugin_id})？该操作不可撤销。`,
    );
    if (!confirmed) {
      return;
    }

    setActivePluginId(plugin.id);
    deleteMutation.mutate(plugin.id);
  };

  return (
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Plugins</h2>
          <p className="ui-page-desc">
            插件列表、ZIP 导入、启用/禁用与删除管理。
          </p>
        </div>
        <button
          type="button"
          className="ui-btn ui-btn-primary ui-focus"
          disabled={isUploading}
          onClick={() => fileInputRef.current?.click()}
        >
          {isUploading ? "导入中..." : "导入插件 ZIP"}
        </button>
      </header>

      <input
        ref={fileInputRef}
        type="file"
        accept=".zip,application/zip"
        className="hidden"
        onChange={(event) => handleImportFile(event.currentTarget.files?.[0] ?? null)}
      />

      <PluginImportCard
        dragging={dragging}
        isUploading={isUploading}
        uploadError={uploadError}
        onRequestSelectFile={() => fileInputRef.current?.click()}
        onDraggingChange={setDragging}
        onImportFile={handleImportFile}
      />

      <PluginListCard
        loading={pluginsQuery.isLoading}
        plugins={plugins}
        expandedPluginId={expandedPluginId}
        activePluginId={activePluginId}
        onToggleExpanded={(pluginId) =>
          setExpandedPluginId((current) => (current === pluginId ? null : pluginId))
        }
        onToggleStatus={handleToggle}
        onDelete={handleDelete}
      />
    </section>
  );
}
