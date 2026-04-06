import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  createProfile,
  deleteProfile,
  fetchProfiles,
  fetchSources,
  rotateProfileExportToken,
  updateProfile,
} from "../../lib/api";
import {
  patchProfileItem,
  removeProfileItem,
  upsertProfileItem,
} from "../../lib/query-cache";
import { queryKeys } from "../../lib/query-keys";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { ProfileItem, ProfileListResponse } from "../../types/core";
import { type ProfileFormMode } from "./constants";
import { ProfileFormCard } from "./profile-form-card";
import { ProfileListCard } from "./profile-list-card";
import { buildSubscriptionUrl, copySubscriptionUrl, formatTimestamp } from "./utils";

export default function ProfilesPage() {
  const queryClient = useQueryClient();
  const addToast = useCoreUiStore((state) => state.addToast);
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);

  const [formMode, setFormMode] = useState<ProfileFormMode>("create");
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);
  const [formName, setFormName] = useState("");
  const [formDescription, setFormDescription] = useState("");
  const [selectedSourceIds, setSelectedSourceIds] = useState<string[]>([]);
  const [activeProfileId, setActiveProfileId] = useState<string | null>(null);

  const baseUrl = status?.baseUrl || "http://127.0.0.1:18118";

  const profilesQuery = useQuery({
    queryKey: queryKeys.profiles.all,
    queryFn: fetchProfiles,
    enabled: phase === "running",
    refetchInterval: eventStreamActive ? 35_000 : 15_000,
  });

  const sourcesQuery = useQuery({
    queryKey: queryKeys.sources.all,
    queryFn: fetchSources,
    enabled: phase === "running",
    refetchInterval: eventStreamActive ? 50_000 : 25_000,
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
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.profiles.all });
      const previousProfiles = queryClient.getQueryData<ProfileListResponse>(
        queryKeys.profiles.all,
      );
      const optimisticProfileId = `optimistic-profile-${Date.now()}`;
      const now = new Date().toISOString();

      queryClient.setQueryData<ProfileListResponse>(queryKeys.profiles.all, (current) =>
        upsertProfileItem(current, {
          profile: {
            id: optimisticProfileId,
            name: input.name,
            description: input.description ?? null,
            created_at: now,
            updated_at: now,
          },
          source_ids: input.sourceIds,
          export_token: null,
        }),
      );

      return { previousProfiles, optimisticProfileId };
    },
    onSuccess: (payload, _input, context) => {
      queryClient.setQueryData<ProfileListResponse>(queryKeys.profiles.all, (current) =>
        upsertProfileItem(
          removeProfileItem(current, context?.optimisticProfileId ?? ""),
          payload.profile,
        ),
      );
      addToast({
        title: "Profile 创建成功",
        description: payload.profile.profile.name,
        variant: "default",
      });
      resetForm();
    },
    onError: (error, _input, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.profiles.all, context.previousProfiles);
      }
      addToast({
        title: "Profile 创建失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.profiles.all });
      }
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
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.profiles.all });
      const previousProfiles = queryClient.getQueryData<ProfileListResponse>(
        queryKeys.profiles.all,
      );
      queryClient.setQueryData<ProfileListResponse | undefined>(queryKeys.profiles.all, (current) =>
        patchProfileItem(current, input.profileId, {
          name: input.name,
          description: input.description ?? null,
          sourceIds: input.sourceIds,
          updatedAt: new Date().toISOString(),
        }),
      );
      return { previousProfiles };
    },
    onSuccess: (payload) => {
      queryClient.setQueryData<ProfileListResponse>(queryKeys.profiles.all, (current) =>
        upsertProfileItem(current, payload.profile),
      );
      addToast({
        title: "Profile 更新成功",
        description: payload.profile.profile.name,
        variant: "default",
      });
    },
    onError: (error, _input, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.profiles.all, context.previousProfiles);
      }
      addToast({
        title: "Profile 更新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.profiles.all });
      }
      setActiveProfileId(null);
    },
  });

  const rotateMutation = useMutation({
    mutationFn: rotateProfileExportToken,
    onSuccess: (payload) => {
      queryClient.setQueryData<ProfileListResponse | undefined>(
        queryKeys.profiles.all,
        (current) =>
        patchProfileItem(current, payload.profile_id, {
          exportToken: payload.token,
          updatedAt: new Date().toISOString(),
        }),
      );
      addToast({
        title: "导出 Token 已轮换",
        description: `旧链接将在 ${formatTimestamp(payload.previous_token_expires_at)} 失效。`,
        variant: "warning",
      });
    },
    onError: (error) => {
      addToast({
        title: "Token 轮换失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.profiles.all });
      }
      setActiveProfileId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteProfile,
    onMutate: async (profileId) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.profiles.all });
      const previousProfiles = queryClient.getQueryData<ProfileListResponse>(
        queryKeys.profiles.all,
      );
      queryClient.setQueryData<ProfileListResponse | undefined>(queryKeys.profiles.all, (current) =>
        removeProfileItem(current, profileId),
      );
      return { previousProfiles };
    },
    onSuccess: () => {
      addToast({
        title: "Profile 已删除",
        description: "关联导出地址已失效。",
        variant: "warning",
      });
      if (formMode === "edit") {
        resetForm();
      }
    },
    onError: (error, _input, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.profiles.all, context.previousProfiles);
      }
      addToast({
        title: "Profile 删除失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      if (!eventStreamActive) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.profiles.all });
      }
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

  const handleCopyUrl = async (profileId: string, format: string, token?: string | null) => {
    if (!token) {
      addToast({
        title: "复制失败",
        description: "当前 Profile 尚未生成导出 token。",
        variant: "error",
      });
      return;
    }

    const url = buildSubscriptionUrl(baseUrl, profileId, format, token);
    const copied = await copySubscriptionUrl(url);
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
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Profiles</h2>
          <p className="ui-page-desc">聚合来源管理、四格式导出地址展示与 token 轮换。</p>
        </div>
        <button type="button" className="ui-btn ui-btn-secondary ui-focus" onClick={resetForm}>
          新建 Profile
        </button>
      </header>

      <ProfileFormCard
        mode={formMode}
        formName={formName}
        formDescription={formDescription}
        selectedSourceIds={selectedSourceIds}
        sourceLoading={sourcesQuery.isLoading}
        sources={sourcesQuery.data?.sources ?? []}
        submitDisabled={submitDisabled}
        submitting={createMutation.isPending || updateMutation.isPending}
        onNameChange={setFormName}
        onDescriptionChange={setFormDescription}
        onToggleSourceSelection={toggleSourceSelection}
        onSubmit={handleSubmit}
        onCancelEdit={resetForm}
      />

      <ProfileListCard
        loading={profilesQuery.isLoading}
        profiles={profiles}
        sourceNameMap={sourceNameMap}
        baseUrl={baseUrl}
        activeProfileId={activeProfileId}
        rotatePending={rotateMutation.isPending}
        deletePending={deleteMutation.isPending}
        onEdit={beginEdit}
        onRotate={handleRotate}
        onDelete={handleDelete}
        onCopyUrl={handleCopyUrl}
      />
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
