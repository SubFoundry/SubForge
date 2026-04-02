import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { Skeleton } from "../../components/skeleton";
import { fetchSystemStatus, refreshAllSources } from "../../lib/api";
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
    <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/50 p-4">
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

  const handleRefreshAll = () => {
    void refreshAllMutation.mutateAsync();
  };

  return (
    <section className="space-y-5">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-2xl font-semibold">Dashboard</h2>
        <div className="flex items-center gap-2">
          <p className="text-sm text-[var(--muted-text)]">Core 连接状态、运行指标与事件概览</p>
          <button
            type="button"
            className="rounded-lg bg-[var(--accent-soft)] px-3 py-2 text-xs font-semibold text-[var(--accent-strong)] transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
            disabled={phase !== "running" || refreshAllMutation.isPending}
            onClick={handleRefreshAll}
          >
            {refreshAllMutation.isPending ? "刷新中..." : "刷新全部"}
          </button>
        </div>
      </header>

      {isBooting ? (
        <div className="grid gap-3 md:grid-cols-4">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : (
        <div className="grid gap-3 md:grid-cols-4">
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

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">连接概览</h3>
        <p className="mt-3 text-sm text-[var(--app-text)]">
          事件流：{eventStreamActive ? "已连接" : "未连接"} | 心跳：{formatTimestamp(heartbeatAt)} |
          最近事件：{lastEvent?.event ?? "暂无"}
        </p>
      </article>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">最近事件（SSE）</h3>
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
    </section>
  );
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return "暂无";
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString("zh-CN", { hour12: false });
}

function isErrorEvent(eventName: string): boolean {
  return (
    eventName.includes("error") ||
    eventName.includes("failed") ||
    eventName.includes("degraded")
  );
}
