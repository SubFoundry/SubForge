import { Skeleton } from "../../components/skeleton";
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
  return (
    <article className="ui-card">
      <div className="ui-card-header">
        <div>
          <h3 className="ui-card-title">来源列表</h3>
          <p className="ui-card-desc">按状态查看来源健康度，并提供刷新、编辑、删除操作。</p>
        </div>
      </div>

      <div className="ui-card-body">
        {loading ? (
          <div className="space-y-2">
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </div>
        ) : sources.length === 0 ? (
          <p className="text-sm text-[var(--muted-text)]">暂无来源，请先创建。</p>
        ) : (
          <div className="space-y-2">
            {sources.map((item) => {
              const source = item.source;
              const busy = activeSourceId === source.id;
              return (
                <article
                  key={source.id}
                  className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/60 px-3 py-3 text-sm"
                >
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="font-medium text-[var(--app-text)]">{source.name}</p>
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        {source.plugin_id} | 创建：{formatTimestamp(source.created_at)} | 更新：
                        {formatTimestamp(source.updated_at)}
                      </p>
                    </div>
                    <div className="flex w-full flex-wrap items-center gap-2 md:w-auto">
                      <span className={`ui-badge ${statusToneClass(source.status)}`}>
                        {source.status}
                      </span>
                      <button
                        type="button"
                        className="ui-btn ui-btn-secondary ui-focus"
                        disabled={busy}
                        onClick={() => onRefresh(source.id)}
                      >
                        刷新
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
                  <pre className="mt-3 overflow-x-auto rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-xs text-[var(--muted-text)]">
                    {JSON.stringify(item.config, null, 2)}
                  </pre>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </article>
  );
}
