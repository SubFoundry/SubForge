import { StatePanel, StateSkeletonRows } from "../../components/state-panel";
import type { SourceListResponse } from "../../types/core";
import { formatTimestamp } from "./utils";
import { statusToneClass } from "../../lib/ui";

type SourceListCardProps = {
  loading: boolean;
  sources: SourceListResponse["sources"];
  activeSourceId: string | null;
  onRefresh: (sourceId: string) => void;
  onEdit: (item: SourceListResponse["sources"][number]) => void;
  onDelete: (item: SourceListResponse["sources"][number]) => void;
};

export function SourceListCard({
  loading,
  sources,
  activeSourceId,
  onRefresh,
  onEdit,
  onDelete,
}: SourceListCardProps) {
  const summary = summarizeSources(sources);

  return (
    <article className="ui-card">
      <div className="ui-card-header">
        <div>
          <h3 className="ui-card-title">来源列表</h3>
          <p className="ui-card-desc">状态优先展示，操作与配置查看按来源卡片就近收敛。</p>
        </div>
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <span className="ui-badge ui-badge-muted">总计 {summary.total}</span>
          <span className="ui-badge ui-badge-success">健康 {summary.healthy}</span>
          <span className="ui-badge ui-badge-warning">运行中/降级 {summary.warn}</span>
          <span className="ui-badge ui-badge-danger">异常 {summary.failed}</span>
        </div>
      </div>

      <div className="ui-card-body">
        {loading ? (
          <StateSkeletonRows rows={3} />
        ) : sources.length === 0 ? (
          <StatePanel
            variant="empty"
            title="还没有来源"
            description="先创建至少一个来源，随后可在此页直接刷新、编辑与删除。"
          />
        ) : (
          <div className="space-y-3">
            {sources.map((item) => {
              const source = item.source;
              const busy = activeSourceId === source.id;
              return (
                <article
                  key={source.id}
                  className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-bg)]/60 px-4 py-3 text-sm shadow-[var(--shadow-soft)]"
                >
                  <div className="flex flex-wrap items-start justify-between gap-4">
                    <div>
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="font-medium text-[var(--app-text)]">{source.name}</p>
                        <span className={`ui-badge ${statusToneClass(source.status)}`}>
                          {source.status}
                        </span>
                      </div>
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        插件：{source.plugin_id}
                      </p>
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        创建：{formatTimestamp(source.created_at)} | 最近更新：
                        {formatTimestamp(source.updated_at)}
                      </p>
                    </div>
                    <div className="flex w-full flex-wrap items-center gap-2 md:w-auto">
                      <button
                        type="button"
                        className="ui-btn ui-btn-primary ui-focus"
                        disabled={busy}
                        onClick={() => onRefresh(source.id)}
                      >
                        {busy ? "执行中..." : "刷新"}
                      </button>
                      <button
                        type="button"
                        className="ui-btn ui-btn-secondary ui-focus"
                        onClick={() => onEdit(item)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="ui-btn ui-btn-danger ui-focus"
                        disabled={busy}
                        onClick={() => onDelete(item)}
                      >
                        删除
                      </button>
                    </div>
                  </div>
                  <details className="mt-3 rounded-lg border border-[var(--panel-border)] bg-[var(--panel-muted)]/30 px-3 py-2">
                    <summary className="cursor-pointer text-xs font-medium text-[var(--muted-text)]">
                      查看配置快照
                    </summary>
                    <pre className="mt-2 overflow-x-auto rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-xs text-[var(--muted-text)]">
                      {JSON.stringify(item.config, null, 2)}
                    </pre>
                  </details>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </article>
  );
}

function summarizeSources(sources: SourceListResponse["sources"]): {
  total: number;
  healthy: number;
  warn: number;
  failed: number;
} {
  let healthy = 0;
  let warn = 0;
  let failed = 0;
  for (const item of sources) {
    const status = item.source.status;
    if (status === "healthy" || status === "enabled" || status === "success") {
      healthy += 1;
      continue;
    }
    if (status === "running" || status === "degraded") {
      warn += 1;
      continue;
    }
    failed += 1;
  }

  return {
    total: sources.length,
    healthy,
    warn,
    failed,
  };
}
