import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Skeleton } from "../../components/skeleton";
import {
  createProfile,
  deleteProfile,
  fetchProfiles,
  fetchSources,
  rotateProfileExportToken,
  updateProfile,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { ProfileItem } from "../../types/core";

type ProfileFormMode = "create" | "edit";

const SUBSCRIPTION_FORMATS = [
  { key: "clash", label: "Clash/Mihomo" },
  { key: "sing-box", label: "sing-box" },
  { key: "base64", label: "Base64" },
  { key: "raw", label: "Raw JSON" },
] as const;

export default function ProfilesPage() {
  const queryClient = useQueryClient();
  const addToast = useCoreUiStore((state) => state.addToast);
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);

  const [formMode, setFormMode] = useState<ProfileFormMode>("create");
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);
  const [formName, setFormName] = useState("");
  const [formDescription, setFormDescription] = useState("");
  const [selectedSourceIds, setSelectedSourceIds] = useState<string[]>([]);
  const [activeProfileId, setActiveProfileId] = useState<string | null>(null);

  const baseUrl = status?.baseUrl || "http://127.0.0.1:18118";

  const profilesQuery = useQuery({
    queryKey: ["profiles"],
    queryFn: fetchProfiles,
    enabled: phase === "running",
    refetchInterval: 15_000,
  });

  const sourcesQuery = useQuery({
    queryKey: ["sources"],
    queryFn: fetchSources,
    enabled: phase === "running",
    refetchInterval: 30_000,
  });

  const sourceNameMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const item of sourcesQuery.data?.sources ?? []) {
      map.set(item.source.id, item.source.name);
    }
    return map;
  }, [sourcesQuery.data?.sources]);

  const createMutation = useMutation({
    mutationFn: createProfile,
    onSuccess: (payload) => {
      addToast({
        title: "Profile 创建成功",
        description: payload.profile.profile.name,
        variant: "default",
      });
      resetForm();
      void queryClient.invalidateQueries({ queryKey: ["profiles"] });
    },
    onError: (error) => {
      addToast({
        title: "Profile 创建失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (input: {
      profileId: string;
      name: string;
      description?: string | null;
      sourceIds: string[];
    }) =>
      updateProfile(input.profileId, {
        name: input.name,
        description: input.description,
        sourceIds: input.sourceIds,
      }),
    onSuccess: (payload) => {
      addToast({
        title: "Profile 更新成功",
        description: payload.profile.profile.name,
        variant: "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["profiles"] });
    },
    onError: (error) => {
      addToast({
        title: "Profile 更新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveProfileId(null);
    },
  });

  const rotateMutation = useMutation({
    mutationFn: rotateProfileExportToken,
    onSuccess: (payload) => {
      addToast({
        title: "导出 Token 已轮换",
        description: `旧链接将在 ${formatTimestamp(payload.previous_token_expires_at)} 失效。`,
        variant: "warning",
      });
      void queryClient.invalidateQueries({ queryKey: ["profiles"] });
    },
    onError: (error) => {
      addToast({
        title: "Token 轮换失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveProfileId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteProfile,
    onSuccess: () => {
      addToast({
        title: "Profile 已删除",
        description: "关联导出地址已失效。",
        variant: "warning",
      });
      if (formMode === "edit") {
        resetForm();
      }
      void queryClient.invalidateQueries({ queryKey: ["profiles"] });
    },
    onError: (error) => {
      addToast({
        title: "Profile 删除失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveProfileId(null);
    },
  });

  const submitDisabled =
    !formName.trim() ||
    createMutation.isPending ||
    updateMutation.isPending ||
    (formMode === "edit" && !editingProfileId);

  const handleSubmit = () => {
    const trimmedName = formName.trim();
    const description = formDescription.trim();

    if (formMode === "create") {
      createMutation.mutate({
        name: trimmedName,
        description: description || undefined,
        sourceIds: selectedSourceIds,
      });
      return;
    }

    if (!editingProfileId) {
      return;
    }
    setActiveProfileId(editingProfileId);
    updateMutation.mutate({
      profileId: editingProfileId,
      name: trimmedName,
      description: description || null,
      sourceIds: selectedSourceIds,
    });
  };

  const beginEdit = (profile: ProfileItem) => {
    setFormMode("edit");
    setEditingProfileId(profile.profile.id);
    setFormName(profile.profile.name);
    setFormDescription(profile.profile.description ?? "");
    setSelectedSourceIds(profile.source_ids);
  };

  const handleDelete = (profile: ProfileItem) => {
    const confirmed = window.confirm(
      `确认删除 Profile "${profile.profile.name}" 吗？该操作会让现有订阅链接失效。`,
    );
    if (!confirmed) {
      return;
    }
    setActiveProfileId(profile.profile.id);
    deleteMutation.mutate(profile.profile.id);
  };

  const handleRotate = (profile: ProfileItem) => {
    const confirmed = window.confirm(
      `确认轮换 Profile "${profile.profile.name}" 的订阅 token 吗？旧链接将在 10 分钟后失效。`,
    );
    if (!confirmed) {
      return;
    }
    setActiveProfileId(profile.profile.id);
    rotateMutation.mutate(profile.profile.id);
  };

  const toggleSourceSelection = (sourceId: string, checked: boolean) => {
    setSelectedSourceIds((current) => {
      if (checked) {
        if (current.includes(sourceId)) {
          return current;
        }
        return [...current, sourceId];
      }
      return current.filter((id) => id !== sourceId);
    });
  };

  const copySubscriptionUrl = async (profileId: string, format: string, token?: string | null) => {
    if (!token) {
      addToast({
        title: "复制失败",
        description: "当前 Profile 尚未生成导出 token。",
        variant: "error",
      });
      return;
    }

    const url = buildSubscriptionUrl(baseUrl, profileId, format, token);
    const copied = await copyText(url);
    if (copied) {
      addToast({
        title: "已复制导出地址",
        description: `${profileId} / ${format}`,
        variant: "default",
      });
    } else {
      addToast({
        title: "复制失败",
        description: "系统剪贴板不可用，请手动复制。",
        variant: "error",
      });
    }
  };

  const profiles = profilesQuery.data?.profiles ?? [];

  return (
    <section className="space-y-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-2xl font-semibold">Profiles</h2>
          <p className="mt-1 text-sm text-[var(--muted-text)]">
            聚合来源管理、四格式导出地址展示与 token 轮换。
          </p>
        </div>
        <button
          type="button"
          className="rounded-lg border border-[var(--panel-border)] px-3 py-2 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
          onClick={resetForm}
        >
          新建 Profile
        </button>
      </header>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">
          {formMode === "create" ? "创建 Profile" : "编辑 Profile"}
        </h3>

        <div className="mt-3 grid gap-3 md:grid-cols-2">
          <label className="text-xs text-[var(--muted-text)]">
            名称
            <input
              className="mt-1 w-full rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]"
              value={formName}
              onChange={(event) => setFormName(event.currentTarget.value)}
              placeholder="例如：主力聚合"
            />
          </label>

          <label className="text-xs text-[var(--muted-text)]">
            描述（可选）
            <input
              className="mt-1 w-full rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]"
              value={formDescription}
              onChange={(event) => setFormDescription(event.currentTarget.value)}
              placeholder="例如：给 Mihomo 与 sing-box 共用"
            />
          </label>
        </div>

        <div className="mt-4 rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 p-3">
          <p className="text-xs text-[var(--muted-text)]">关联来源（可多选）</p>
          {sourcesQuery.isLoading ? (
            <div className="mt-2 space-y-2">
              <Skeleton className="h-8" />
              <Skeleton className="h-8" />
            </div>
          ) : (sourcesQuery.data?.sources ?? []).length === 0 ? (
            <p className="mt-2 text-sm text-[var(--muted-text)]">暂无来源，请先在 Sources 页面创建。</p>
          ) : (
            <div className="mt-2 grid gap-2 md:grid-cols-2">
              {(sourcesQuery.data?.sources ?? []).map((item) => (
                <label
                  key={item.source.id}
                  className="flex items-center gap-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]"
                >
                  <input
                    type="checkbox"
                    checked={selectedSourceIds.includes(item.source.id)}
                    onChange={(event) =>
                      toggleSourceSelection(item.source.id, event.currentTarget.checked)
                    }
                  />
                  <span>{item.source.name}</span>
                </label>
              ))}
            </div>
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="rounded-lg bg-[var(--accent-soft)] px-3 py-2 text-xs font-semibold text-[var(--accent-strong)] transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
            disabled={submitDisabled}
            onClick={handleSubmit}
          >
            {createMutation.isPending || updateMutation.isPending
              ? "提交中..."
              : formMode === "create"
                ? "创建 Profile"
                : "保存修改"}
          </button>
          {formMode === "edit" && (
            <button
              type="button"
              className="rounded-lg border border-[var(--panel-border)] px-3 py-2 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
              onClick={resetForm}
            >
              取消编辑
            </button>
          )}
        </div>
      </article>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">Profile 列表</h3>

        {profilesQuery.isLoading ? (
          <div className="mt-3 space-y-3">
            <Skeleton className="h-32" />
            <Skeleton className="h-32" />
          </div>
        ) : profiles.length === 0 ? (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无 Profile，请先创建。</p>
        ) : (
          <div className="mt-3 space-y-3">
            {profiles.map((item) => {
              const profile = item.profile;
              const busy = activeProfileId === profile.id;
              return (
                <article
                  key={profile.id}
                  className="rounded-lg border border-[var(--panel-border)] bg-[var(--panel-bg)]/55 p-3"
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
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)]"
                        onClick={() => beginEdit(item)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="rounded-md border border-amber-400/35 px-2 py-1 text-xs text-amber-300 transition hover:bg-amber-500/15 disabled:cursor-not-allowed disabled:opacity-60"
                        disabled={busy}
                        onClick={() => handleRotate(item)}
                      >
                        {busy && rotateMutation.isPending ? "轮换中..." : "轮换 Token"}
                      </button>
                      <button
                        type="button"
                        className="rounded-md border border-rose-400/35 px-2 py-1 text-xs text-rose-300 transition hover:bg-rose-500/15 disabled:cursor-not-allowed disabled:opacity-60"
                        disabled={busy}
                        onClick={() => handleDelete(item)}
                      >
                        {busy && deleteMutation.isPending ? "删除中..." : "删除"}
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
                          <code className="overflow-x-auto text-xs text-[var(--app-text)]">{url}</code>
                          <button
                            type="button"
                            className="rounded-md border border-[var(--panel-border)] px-2 py-1 text-xs text-[var(--app-text)] transition hover:bg-[var(--panel-bg)] disabled:cursor-not-allowed disabled:opacity-60"
                            disabled={!item.export_token}
                            onClick={() =>
                              copySubscriptionUrl(profile.id, format.key, item.export_token)
                            }
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
      </article>
    </section>
  );

  function resetForm() {
    setFormMode("create");
    setEditingProfileId(null);
    setFormName("");
    setFormDescription("");
    setSelectedSourceIds([]);
  }
}

function buildSubscriptionUrl(
  baseUrl: string,
  profileId: string,
  format: string,
  token: string,
): string {
  const normalizedBase = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
  return `${normalizedBase}/api/profiles/${encodeURIComponent(profileId)}/${format}?token=${encodeURIComponent(token)}`;
}

async function copyText(value: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return true;
    } catch {
      // 忽略并降级到 textarea 方案。
    }
  }

  try {
    const textarea = document.createElement("textarea");
    textarea.value = value;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "absolute";
    textarea.style.left = "-9999px";
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(textarea);
    return copied;
  } catch {
    return false;
  }
}

function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString("zh-CN", { hour12: false });
}
