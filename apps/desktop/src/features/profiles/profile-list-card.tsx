import { Skeleton } from "../../components/skeleton";
import type { ProfileItem } from "../../types/core";
import { SUBSCRIPTION_FORMATS } from "./constants";
import { buildSubscriptionUrl, formatTimestamp } from "./utils";

type ProfileListCardProps = {
  loading: boolean;
  profiles: ProfileItem[];
  sourceNameMap: Map<string, string>;
  baseUrl: string;
  activeProfileId: string | null;
  rotatePending: boolean;
  deletePending: boolean;
  onEdit: (profile: ProfileItem) => void;
  onRotate: (profile: ProfileItem) => void;
  onDelete: (profile: ProfileItem) => void;
  onCopyUrl: (profileId: string, format: string, token?: string | null) => void;
};

export function ProfileListCard({
  loading,
  profiles,
  sourceNameMap,
  baseUrl,
  activeProfileId,
  rotatePending,
  deletePending,
  onEdit,
  onRotate,
  onDelete,
  onCopyUrl,
}: ProfileListCardProps) {
  return (
    <article className="ui-card">
      <div className="ui-card-header">
        <div>
          <h3 className="ui-card-title">Profile 列表</h3>
          <p className="ui-card-desc">每个 Profile 同时展示四种导出格式与复制入口。</p>
        </div>
      </div>

      <div className="ui-card-body">
        {loading ? (
          <div className="space-y-3">
            <Skeleton className="h-32" />
            <Skeleton className="h-32" />
          </div>
        ) : profiles.length === 0 ? (
          <p className="text-sm text-[var(--muted-text)]">暂无 Profile，请先创建。</p>
        ) : (
          <div className="space-y-3">
            {profiles.map((item) => {
              const profile = item.profile;
              const busy = activeProfileId === profile.id;
              return (
                <article
                  key={profile.id}
                  className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/60 p-3"
                >
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="font-medium text-[var(--app-text)]">{profile.name}</p>
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        ID: {profile.id} | 来源数：{item.source_ids.length} | 更新：
                        {formatTimestamp(profile.updated_at)}
                      </p>
                      {profile.description && (
                        <p className="mt-1 text-xs text-[var(--muted-text)]">{profile.description}</p>
                      )}
                      <p className="mt-1 text-xs text-[var(--muted-text)]">
                        来源：
                        {item.source_ids.length === 0
                          ? "未关联来源"
                          : item.source_ids
                              .map((sourceId) => sourceNameMap.get(sourceId) ?? sourceId)
                              .join(" / ")}
                      </p>
                    </div>
                    <div className="flex w-full flex-wrap items-center gap-2 md:w-auto">
                      <button
                        type="button"
                        className="ui-btn ui-btn-secondary ui-focus"
                        onClick={() => onEdit(item)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="ui-btn ui-btn-secondary ui-focus"
                        disabled={busy}
                        onClick={() => onRotate(item)}
                      >
                        {busy && rotatePending ? "轮换中..." : "轮换 Token"}
                      </button>
                      <button
                        type="button"
                        className="ui-btn ui-btn-danger ui-focus"
                        disabled={busy}
                        onClick={() => onDelete(item)}
                      >
                        {busy && deletePending ? "删除中..." : "删除"}
                      </button>
                    </div>
                  </div>

                  <div className="mt-3 space-y-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-3">
                    {SUBSCRIPTION_FORMATS.map((format) => {
                      const url = item.export_token
                        ? buildSubscriptionUrl(baseUrl, profile.id, format.key, item.export_token)
                        : "未生成 token";
                      return (
                        <div
                          key={format.key}
                          className="grid gap-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-muted)]/25 px-2 py-2 md:grid-cols-[140px_1fr_auto] md:items-center"
                        >
                          <span className="text-xs font-medium text-[var(--muted-text)]">
                            {format.label}
                          </span>
                          <code className="break-all text-xs text-[var(--app-text)]">{url}</code>
                          <button
                            type="button"
                            className="ui-btn ui-btn-secondary ui-focus"
                            disabled={!item.export_token}
                            onClick={() => onCopyUrl(profile.id, format.key, item.export_token)}
                          >
                            复制
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </article>
  );
}
