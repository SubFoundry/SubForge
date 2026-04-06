import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { StatePanel, StateSkeletonRows } from "../../components/state-panel";
import { fetchRefreshLogs, fetchSources } from "../../lib/api";
import { queryKeys } from "../../lib/query-keys";
import { formatTimestamp, statusToneClass } from "../../lib/ui";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { RefreshLog } from "../../types/core";

const PAGE_SIZE = 12;

export default function RunsPage() {
  const phase = useCoreUiStore((state) => state.phase);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const [statusFilter, setStatusFilter] = useState<"all" | "running" | "success" | "failed">(
    "all",
  );
  const [sourceFilter, setSourceFilter] = useState<string>("all");
  const [page, setPage] = useState(1);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    setPage(1);
  }, [statusFilter, sourceFilter]);

  const sourcesQuery = useQuery({
    queryKey: queryKeys.runs.sources,
    queryFn: fetchSources,
    enabled: phase === "running",
    refetchInterval: eventStreamActive ? 60_000 : 20_000,
  });

  const logsQuery = useQuery({
    queryKey: queryKeys.runs.logs({
      status: statusFilter,
      source: sourceFilter,
      page,
    }),
    queryFn: () =>
      fetchRefreshLogs({
        limit: PAGE_SIZE,
        offset: (page - 1) * PAGE_SIZE,
        status: statusFilter === "all" ? undefined : statusFilter,
        sourceId: sourceFilter === "all" ? undefined : sourceFilter,
        includeScriptLogs: true,
      }),
    enabled: phase === "running",
    refetchInterval:
      statusFilter === "running" ? 5_000 : eventStreamActive ? 20_000 : 10_000,
  });

  const sourceOptions = useMemo(
    () =>
      (sourcesQuery.data?.sources ?? [])
        .map((item) => ({
          id: item.source.id,
          label: item.source.name,
        }))
        .sort((a, b) => a.label.localeCompare(b.label, "zh-CN")),
    [sourcesQuery.data?.sources],
  );

  const logs = logsQuery.data?.logs ?? [];
  const pagination = logsQuery.data?.pagination ?? {
    limit: PAGE_SIZE,
    offset: 0,
    total: 0,
    hasMore: false,
  };
  const totalPages = Math.max(1, Math.ceil(pagination.total / PAGE_SIZE));
  const hasPreviousPage = page > 1;
  const hasNextPage = pagination.hasMore;
  const summary = useMemo(() => summarizeLogs(logs), [logs]);

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  useEffect(() => {
    if (!expandedId) {
      return;
    }
    const expandedLog = logs.find((item) => item.id === expandedId);
    if (!expandedLog || !hasRunDetails(expandedLog)) {
      setExpandedId(null);
    }
  }, [expandedId, logs]);

  return (
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Runs</h2>
          <p className="ui-page-desc">
            按来源/状态筛选刷新记录，时间按最新优先排序并支持分页浏览。
          </p>
        </div>
      </header>

      <article className="ui-card">
        <div className="grid gap-3 md:grid-cols-3">
          <label className="text-xs text-[var(--muted-text)]">
            状态筛选
            <select
              className="ui-select ui-focus mt-1"
              value={statusFilter}
              onChange={(event) =>
                setStatusFilter(event.currentTarget.value as typeof statusFilter)
              }
            >
              <option value="all">全部</option>
              <option value="running">running</option>
              <option value="success">success</option>
              <option value="failed">failed</option>
            </select>
          </label>

          <label className="text-xs text-[var(--muted-text)]">
            来源筛选
            <select
              className="ui-select ui-focus mt-1"
              value={sourceFilter}
              onChange={(event) => setSourceFilter(event.currentTarget.value)}
            >
              <option value="all">全部来源</option>
              {sourceOptions.map((source) => (
                <option key={source.id} value={source.id}>
                  {source.label}
                </option>
              ))}
            </select>
          </label>

          <div className="text-xs text-[var(--muted-text)]">
            记录总数
            <p className="mt-1 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]">
              {pagination.total} 条
            </p>
          </div>
        </div>
      </article>

      <article className="grid gap-3 md:grid-cols-3">
        <MetricCard label="当前页运行中" value={summary.running} />
        <MetricCard label="当前页成功" value={summary.success} />
        <MetricCard label="当前页失败" value={summary.failed} />
      </article>

      <article className="ui-card">
        {logsQuery.isLoading ? (
          <StateSkeletonRows rows={3} />
        ) : logsQuery.isError ? (
          <StatePanel
            variant="error"
            title="运行记录加载失败"
            description={
              logsQuery.error instanceof Error
                ? logsQuery.error.message
                : "请稍后重试或检查 Core 状态。"
            }
          />
        ) : logs.length === 0 ? (
          <StatePanel
            variant="empty"
            title="没有匹配记录"
            description="可调整状态筛选、来源筛选或切回第一页查看最近任务。"
          />
        ) : (
          <div className="space-y-3">
            {logs.map((log) => (
              <RunItem
                key={log.id}
                log={log}
                expanded={expandedId === log.id}
                onToggle={() =>
                  setExpandedId((current) => (current === log.id ? null : log.id))
                }
              />
            ))}
          </div>
        )}
      </article>

      <footer className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/55 px-4 py-3 text-sm">
        <span className="text-[var(--muted-text)]">
          第 {page} / {totalPages} 页（每页 {PAGE_SIZE} 条）
        </span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="ui-btn ui-btn-secondary ui-focus"
            disabled={!hasPreviousPage}
            onClick={() => setPage((value) => Math.max(1, value - 1))}
          >
            上一页
          </button>
          <button
            type="button"
            className="ui-btn ui-btn-secondary ui-focus"
            disabled={!hasNextPage}
            onClick={() => setPage((value) => value + 1)}
          >
            下一页
          </button>
        </div>
      </footer>
    </section>
  );
}

function RunItem({
  log,
  expanded,
  onToggle,
}: {
  log: RefreshLog;
  expanded: boolean;
  onToggle: () => void;
}) {
  const durationText = formatDuration(log.startedAt, log.finishedAt);
  const hasScriptLogs = log.scriptLogs.length > 0;
  const hasDetails = hasRunDetails(log);

  return (
    <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-4 py-3 text-sm shadow-[var(--shadow-soft)]">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <p className="font-medium text-[var(--app-text)]">{log.sourceName ?? log.sourceId}</p>
            <span className={`ui-badge ${statusToneClass(log.status)}`}>{log.status}</span>
          </div>
          <p className="mt-1 text-xs text-[var(--muted-text)]">
            触发：{log.triggerType}
          </p>
          <p className="mt-1 text-xs text-[var(--muted-text)]">
            开始：{formatTimestamp(log.startedAt)} | 结束：{formatTimestamp(log.finishedAt)}
          </p>
          <p className="mt-1 text-xs text-[var(--muted-text)]">
            节点：{log.nodeCount ?? "-"} | 耗时：{durationText}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {hasDetails && (
            <button
              type="button"
              className="ui-btn ui-btn-secondary ui-focus"
              onClick={onToggle}
            >
              {expanded ? "收起详情" : "查看详情"}
            </button>
          )}
        </div>
      </div>
      {expanded && hasDetails && (
        <div className="mt-3 space-y-2 text-xs">
          {log.status === "failed" && (
            <div className="rounded-md border border-rose-500/35 bg-rose-500/10 px-3 py-2">
              <p className="font-medium text-rose-300">{log.errorCode ?? "E_INTERNAL"}</p>
              <p className="mt-1 text-[var(--app-text)]">{log.errorMessage ?? "未知错误"}</p>
            </div>
          )}
          {hasScriptLogs ? (
            <div className="rounded-md border border-[var(--panel-border)] bg-[var(--panel-muted)]/30 px-3 py-2">
              <p className="font-medium text-[var(--app-text)]">
                脚本日志（{log.scriptLogs.length}）
              </p>
              <ul className="mt-2 space-y-2">
                {log.scriptLogs.map((item) => (
                  <li
                    key={item.id}
                    className="rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-2 py-2"
                  >
                    <p className="font-medium text-[var(--app-text)]">
                      {item.level.toUpperCase()} · {formatTimestamp(item.createdAt)}
                    </p>
                    <p className="mt-1 text-[var(--muted-text)]">
                      插件：{item.pluginId} | 来源：{item.sourceId}
                    </p>
                    <p className="mt-1 whitespace-pre-wrap break-words text-[var(--app-text)]">
                      {item.message}
                    </p>
                  </li>
                ))}
              </ul>
            </div>
          ) : (
            <div className="rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-3 py-2 text-[var(--muted-text)]">
              该任务没有脚本日志。
            </div>
          )}
        </div>
      )}
    </article>
  );
}

function hasRunDetails(log: RefreshLog): boolean {
  return log.status === "failed" || log.scriptLogs.length > 0;
}

function formatDuration(
  startedAt: string | null | undefined,
  finishedAt: string | null | undefined,
): string {
  if (!startedAt || !finishedAt) {
    return "-";
  }
  const start = new Date(startedAt).getTime();
  const finish = new Date(finishedAt).getTime();
  if (Number.isNaN(start) || Number.isNaN(finish) || finish < start) {
    return "-";
  }
  const seconds = Math.floor((finish - start) / 1000);
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remain = seconds % 60;
  return `${minutes}m ${remain}s`;
}

function summarizeLogs(logs: RefreshLog[]): {
  running: number;
  success: number;
  failed: number;
} {
  let running = 0;
  let success = 0;
  let failed = 0;
  for (const log of logs) {
    if (log.status === "running") {
      running += 1;
      continue;
    }
    if (log.status === "success") {
      success += 1;
      continue;
    }
    if (log.status === "failed") {
      failed += 1;
    }
  }
  return { running, success, failed };
}

function MetricCard({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 px-4 py-3 shadow-[var(--shadow-soft)]">
      <p className="text-xs uppercase tracking-wide text-[var(--muted-text)]">{label}</p>
      <p className="mt-2 text-2xl font-semibold text-[var(--app-text)]">{value}</p>
    </div>
  );
}
