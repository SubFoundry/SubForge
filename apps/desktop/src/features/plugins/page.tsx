import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { Skeleton } from "../../components/skeleton";
import {
  deletePlugin,
  fetchPlugins,
  importPluginZip,
  togglePlugin,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { PluginRecord } from "../../types/core";

export default function PluginsPage() {
  const queryClient = useQueryClient();
  const addToast = useCoreUiStore((state) => state.addToast);
  const phase = useCoreUiStore((state) => state.phase);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [expandedPluginId, setExpandedPluginId] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [activePluginId, setActivePluginId] = useState<string | null>(null);

  const pluginsQuery = useQuery({
    queryKey: ["plugins"],
    queryFn: fetchPlugins,
    refetchInterval: 15_000,
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
      void queryClient.invalidateQueries({ queryKey: ["plugins"] });
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
      void queryClient.invalidateQueries({ queryKey: ["plugins"] });
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
      void queryClient.invalidateQueries({ queryKey: ["plugins"] });
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
    <section className="space-y-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-2xl font-semibold">Plugins</h2>
          <p className="mt-1 text-sm text-[var(--muted-text)]">
            插件列表、ZIP 导入、启用/禁用与删除管理。
          </p>
        </div>
        <button
          type="button"
          className="rounded-lg bg-[var(--accent-soft)] px-3 py-2 text-xs font-semibold text-[var(--accent-strong)] transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
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

      <div
        className={`rounded-xl border border-dashed p-4 transition ${
          dragging
            ? "border-[var(--accent-strong)] bg-[var(--accent-soft)]/25"
            : "border-[var(--panel-border)] bg-[var(--panel-muted)]/35"
        }`}
        onDragEnter={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(event) => {
          event.preventDefault();
          setDragging(false);
        }}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          handleImportFile(event.dataTransfer.files?.[0] ?? null);
        }}
      >
        <p className="text-sm text-[var(--app-text)]">
          拖拽 `.zip` 插件包到此处，或使用“导入插件 ZIP”按钮选择文件。
        </p>
        <p className="mt-1 text-xs text-[var(--muted-text)]">非法插件包会显示错误详情。</p>
        {uploadError && (
          <p className="mt-2 rounded-md border border-rose-400/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
            {uploadError}
          </p>
        )}
      </div>

      {pluginsQuery.isLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
        </div>
      ) : plugins.length === 0 ? (
        <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4 text-sm text-[var(--muted-text)]">
          暂无插件。请先导入插件包。
        </article>
      ) : (
        <div className="space-y-3">
          {plugins.map((plugin) => {
            const expanded = expandedPluginId === plugin.id;
            const busy = activePluginId === plugin.id;
            return (
              <article
                key={plugin.id}
                className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h3 className="text-base font-semibold text-[var(--app-text)]">{plugin.name}</h3>
                    <p className="mt-1 text-xs text-[var(--muted-text)]">{plugin.plugin_id}</p>
                    <p className="mt-1 text-xs text-[var(--muted-text)]">
                      版本 {plugin.version} · spec {plugin.spec_version} · 类型{" "}
                      {plugin.plugin_type}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <span
                      className={`rounded-full px-2 py-1 text-xs ${
                        plugin.status === "enabled"
                          ? "bg-emerald-500/20 text-emerald-300"
                          : "bg-amber-500/20 text-amber-300"
                      }`}
                    >
                      {plugin.status}
                    </span>
                    <button
                      type="button"
                      className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)] disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() =>
                        setExpandedPluginId(expanded ? null : plugin.id)
                      }
                    >
                      {expanded ? "收起详情" : "查看详情"}
                    </button>
                    <button
                      type="button"
                      className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)] disabled:cursor-not-allowed disabled:opacity-60"
                      disabled={busy}
                      onClick={() => handleToggle(plugin)}
                    >
                      {busy
                        ? "处理中..."
                        : plugin.status === "enabled"
                          ? "禁用"
                          : "启用"}
                    </button>
                    <button
                      type="button"
                      className="rounded-md border border-rose-400/35 px-2 py-1 text-xs text-rose-300 transition hover:bg-rose-500/15 disabled:cursor-not-allowed disabled:opacity-60"
                      disabled={busy}
                      onClick={() => handleDelete(plugin)}
                    >
                      删除
                    </button>
                  </div>
                </div>

                {expanded && (
                  <dl className="mt-3 grid gap-2 rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 p-3 text-xs md:grid-cols-2">
                    <DetailRow label="插件记录 ID" value={plugin.id} />
                    <DetailRow label="插件标识" value={plugin.plugin_id} />
                    <DetailRow label="类型" value={plugin.plugin_type} />
                    <DetailRow label="状态" value={plugin.status} />
                    <DetailRow
                      label="安装时间"
                      value={formatTimestamp(plugin.installed_at)}
                    />
                    <DetailRow
                      label="更新时间"
                      value={formatTimestamp(plugin.updated_at)}
                    />
                  </dl>
                )}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[var(--muted-text)]">{label}</dt>
      <dd className="mt-1 break-all text-[var(--app-text)]">{value}</dd>
    </div>
  );
}

function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString("zh-CN", { hour12: false });
}
