import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { Skeleton } from "../../components/skeleton";
import { formatTimestamp } from "../../lib/ui";
import {
  fetchRefreshLogs,
  fetchSystemStatus,
  refreshAllSources,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";

function StatusCard({
  title,
  value,
  hint,
}: {
  title: string;
  value: string;
  hint: string;
}) {
  return (
    <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-bg)]/65 p-4">
      <p className="text-xs uppercase tracking-wide text-[var(--muted-text)]">{title}</p>
      <p className="mt-2 text-xl font-semibold text-[var(--app-text)]">{value}</p>
      <p className="mt-1 text-xs text-[var(--muted-text)]">{hint}</p>
    </article>
  );
}

export default function DashboardPage() {
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);
  const heartbeatAt = useCoreUiStore((state) => state.heartbeatAt);
  const lastEvent = useCoreUiStore((state) => state.lastEvent);
  const eventHistory = useCoreUiStore((state) => state.eventHistory);
  const lastRefreshAtFromEvents = useCoreUiStore((state) => state.lastRefreshAt);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const error = useCoreUiStore((state) => state.error);
  const addToast = useCoreUiStore((state) => state.addToast);
  const queryClient = useQueryClient();

  const systemStatusQuery = useQuery({
    queryKey: ["dashboard-system-status", lastRefreshAtFromEvents],
    queryFn: fetchSystemStatus,
    refetchInterval: 15_000,
    enabled: status?.running === true,
  });

  const recentLogsQuery = useQuery({
    queryKey: ["dashboard-logs", "recent"],
    queryFn: () => fetchRefreshLogs({ limit: 10 }),
    refetchInterval: 15_000,
    enabled: status?.running === true,
  });

  const recentErrorLogsQuery = useQuery({
    queryKey: ["dashboard-logs", "failed"],
    queryFn: () => fetchRefreshLogs({ limit: 5, status: "failed" }),
    refetchInterval: 15_000,
    enabled: status?.running === true,
  });

  const refreshAllMutation = useMutation({
    mutationFn: refreshAllSources,
    onSuccess: (result) => {
      const failedCount = result.failed.length;
      const successCount = result.succeeded.length;
      addToast({
        title: failedCount > 0 ? "刷新全部完成（部分失败）" : "刷新全部完成",
        description:
          failedCount > 0
            ? `成功 ${successCount} 个，失败 ${failedCount} 个。`
            : `已触发 ${successCount} 个来源刷新任务。`,
        variant: failedCount > 0 ? "warning" : "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (mutationError) => {
      addToast({
        title: "刷新全部失败",
        description:
          mutationError instanceof Error ? mutationError.message : "调用来源刷新接口失败。",
        variant: "error",
      });
    },
  });

  const isBooting = phase === "booting" || (phase === "idle" && !status);
  const displayLastRefreshAt =
    systemStatusQuery.data?.lastRefreshAt ?? lastRefreshAtFromEvents ?? null;

  const recentEvents = useMemo(() => eventHistory.slice(0, 8), [eventHistory]);
  const recentLogs = recentLogsQuery.data?.logs ?? [];
  const recentErrorLogs = recentErrorLogsQuery.data?.logs ?? [];

  const handleRefreshAll = () => {
    void refreshAllMutation.mutateAsync();
  };

  return (
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Dashboard</h2>
          <p className="ui-page-desc">核心状态、刷新指标、事件与错误统一浏览。</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="ui-btn ui-btn-primary ui-focus"
            disabled={phase !== "running" || refreshAllMutation.isPending}
            onClick={handleRefreshAll}
          >
            {refreshAllMutation.isPending ? "刷新中..." : "刷新全部"}
          </button>
        </div>
      </header>

      {isBooting ? (
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <StatusCard
            title="Core 状态"
            value={status?.running ? "运行中" : "未运行"}
            hint={status?.baseUrl ?? "-"}
          />
          <StatusCard
            title="活跃来源数"
            value={String(systemStatusQuery.data?.activeSources ?? 0)}
            hint="status != disabled"
          />
          <StatusCard
            title="总节点数"
            value={String(systemStatusQuery.data?.totalNodes ?? 0)}
            hint="来自来源缓存"
          />
          <StatusCard
            title="最后刷新"
            value={formatTimestamp(displayLastRefreshAt)}
            hint={error ?? "由刷新事件与系统状态联合推导"}
          />
        </div>
      )}

      <article className="ui-card">
        <div className="ui-card-header">
          <div>
            <h3 className="ui-card-title">连接概览</h3>
            <p className="ui-card-desc">事件流、心跳与最近事件的即时状态。</p>
          </div>
        </div>
        <div className="ui-card-body">
          <p className="text-sm text-[var(--app-text)]">
            事件流：{eventStreamActive ? "已连接" : "未连接"} | 心跳：
            {formatTimestamp(heartbeatAt)} | 最近事件：{lastEvent?.event ?? "暂无"}
          </p>
        </div>
      </article>

      <article className="ui-card">
        <h3 className="ui-card-title">最近事件（SSE）</h3>
        {recentEvents.length > 0 ? (
          <ul className="mt-3 space-y-2 text-sm">
            {recentEvents.map((event) => (
              <li
                key={`${event.timestamp ?? "no-ts"}-${event.event}-${event.sourceId ?? "no-src"}`}
                className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-3 py-2"
              >
                <p
                  className={`font-medium ${
                    isErrorEvent(event.event)
                      ? "text-rose-300"
                      : "text-[var(--accent-strong)]"
                  }`}
                >
                  {event.event}
                </p>
                <p className="text-[var(--app-text)]">{event.message}</p>
                <p className="text-xs text-[var(--muted-text)]">
                  source: {event.sourceId ?? "-"} | timestamp:{" "}
                  {formatTimestamp(event.timestamp ?? null)}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无事件。</p>
        )}
      </article>

      <article className="ui-card">
        <h3 className="ui-card-title">最近刷新</h3>
        {recentLogsQuery.isLoading ? (
          <div className="mt-3 space-y-2">
            <Skeleton className="h-16" />
            <Skeleton className="h-16" />
          </div>
        ) : recentLogs.length > 0 ? (
          <ul className="mt-3 space-y-2 text-sm">
            {recentLogs.map((log) => (
              <li
                key={log.id}
                className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-3 py-2"
              >
                <p className="font-medium text-[var(--accent-strong)]">
                  {log.sourceName ?? log.sourceId} · {log.status}
                </p>
                <p className="text-[var(--app-text)]">
                  触发：{log.triggerType} | 节点：{log.nodeCount ?? "-"}
                </p>
                <p className="text-xs text-[var(--muted-text)]">
                  开始：{formatTimestamp(log.startedAt)} | 结束：{formatTimestamp(log.finishedAt)}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无刷新记录。</p>
        )}
      </article>

      <article className="ui-card">
        <h3 className="ui-card-title">最近错误</h3>
        {recentErrorLogsQuery.isLoading ? (
          <div className="mt-3 space-y-2">
            <Skeleton className="h-16" />
          </div>
        ) : recentErrorLogs.length > 0 ? (
          <ul className="mt-3 space-y-2 text-sm">
            {recentErrorLogs.map((log) => (
              <li
                key={log.id}
                className="rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-2"
              >
                <p className="font-medium text-rose-300">
                  {log.sourceName ?? log.sourceId} · {log.errorCode ?? "E_INTERNAL"}
                </p>
                <p className="text-[var(--app-text)]">{log.errorMessage ?? "未知错误"}</p>
                <p className="text-xs text-[var(--muted-text)]">
                  时间：{formatTimestamp(log.finishedAt ?? log.startedAt)}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无错误记录。</p>
        )}
      </article>
    </section>
  );
}

function isErrorEvent(eventName: string): boolean {
  return (
    eventName.includes("error") ||
    eventName.includes("failed") ||
    eventName.includes("degraded")
  );
}
