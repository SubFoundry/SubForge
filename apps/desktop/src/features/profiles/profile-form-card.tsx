import { Skeleton } from "../../components/skeleton";
import type { SourceListResponse } from "../../types/core";
import type { ProfileFormMode } from "./constants";

type ProfileFormCardProps = {
  mode: ProfileFormMode;
  formName: string;
  formDescription: string;
  selectedSourceIds: string[];
  sourceLoading: boolean;
  sources: SourceListResponse["sources"];
  routingTemplateSourceId: string | null;
  submitDisabled: boolean;
  submitting: boolean;
  onNameChange: (value: string) => void;
  onDescriptionChange: (value: string) => void;
  onToggleSourceSelection: (sourceId: string, checked: boolean) => void;
  onRoutingTemplateSourceChange: (sourceId: string | null) => void;
  onSubmit: () => void;
  onCancelEdit: () => void;
};

export function ProfileFormCard({
  mode,
  formName,
  formDescription,
  selectedSourceIds,
  sourceLoading,
  sources,
  routingTemplateSourceId,
  submitDisabled,
  submitting,
  onNameChange,
  onDescriptionChange,
  onToggleSourceSelection,
  onRoutingTemplateSourceChange,
  onSubmit,
  onCancelEdit,
}: ProfileFormCardProps) {
  return (
    <article className="ui-card">
      <div className="ui-card-header">
        <div>
          <h3 className="ui-card-title">{mode === "create" ? "创建 Profile" : "编辑 Profile"}</h3>
          <p className="ui-card-desc">统一管理来源绑定与导出配置。</p>
        </div>
      </div>

      <div className="ui-card-body space-y-4">
        <div className="grid gap-3 md:grid-cols-2">
          <label className="text-xs text-[var(--muted-text)]">
            <span className="text-[var(--app-text)]">名称</span>
            <input
              className="ui-input ui-focus mt-1"
              value={formName}
              onChange={(event) => onNameChange(event.currentTarget.value)}
              placeholder="例如：主力聚合"
            />
          </label>

          <label className="text-xs text-[var(--muted-text)]">
            <span className="text-[var(--app-text)]">描述（可选）</span>
            <input
              className="ui-input ui-focus mt-1"
              value={formDescription}
              onChange={(event) => onDescriptionChange(event.currentTarget.value)}
              placeholder="例如：给 Mihomo 与 sing-box 共用"
            />
          </label>
        </div>

        <div className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/65 p-3">
          <p className="text-xs text-[var(--muted-text)]">关联来源（可多选）</p>
          {sourceLoading ? (
            <div className="mt-2 space-y-2">
              <Skeleton className="h-8" />
              <Skeleton className="h-8" />
            </div>
          ) : sources.length === 0 ? (
            <p className="mt-2 text-sm text-[var(--muted-text)]">暂无来源，请先在 Sources 页面创建。</p>
          ) : (
            <div className="mt-2 grid gap-2 md:grid-cols-2">
              {sources.map((item) => (
                <label
                  key={item.source.id}
                  className="ui-focus flex items-center gap-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]"
                >
                  <input
                    className="ui-focus"
                    type="checkbox"
                    checked={selectedSourceIds.includes(item.source.id)}
                    onChange={(event) =>
                      onToggleSourceSelection(item.source.id, event.currentTarget.checked)
                    }
                  />
                  <span>{item.source.name}</span>
                </label>
              ))}
            </div>
          )}
        </div>

        <label className="text-xs text-[var(--muted-text)]">
          <span className="text-[var(--app-text)]">分流模板来源（可选）</span>
          <select
            className="ui-input ui-focus mt-1"
            value={routingTemplateSourceId ?? ""}
            onChange={(event) =>
              onRoutingTemplateSourceChange(event.currentTarget.value || null)
            }
          >
            <option value="">不使用模板（默认分组）</option>
            {sources
              .filter((item) => selectedSourceIds.includes(item.source.id))
              .map((item) => (
                <option key={item.source.id} value={item.source.id}>
                  {item.source.name}
                </option>
              ))}
          </select>
          <p className="mt-1 text-[11px] text-[var(--muted-text)]">
            用于 Clash / sing-box 模板导出：沿用该来源的分组结构，并注入当前 Profile 的最终节点集。
          </p>
        </label>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="ui-btn ui-btn-primary ui-focus"
            disabled={submitDisabled}
            onClick={onSubmit}
          >
            {submitting ? "提交中..." : mode === "create" ? "创建 Profile" : "保存修改"}
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
