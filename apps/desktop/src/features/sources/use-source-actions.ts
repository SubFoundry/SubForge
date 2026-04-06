import { useMutation, type QueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { createSource, deleteSource, refreshSource, updateSource } from "../../lib/api";
import type { InlineActionState } from "../../components/inline-action-feedback";
import {
  patchSourceItem,
  patchSystemStatus,
  removeSourceItem,
  upsertSourceItem,
} from "../../lib/query-cache";
import { queryKeys } from "../../lib/query-keys";
import type { ToastMessage } from "../../stores/core-ui-store";
import type { SourceListResponse, SystemStatusResponse } from "../../types/core";

type UseSourceActionsOptions = {
  queryClient: QueryClient;
  addToast: (toast: Omit<ToastMessage, "id">) => string;
  eventDrivenSyncEnabled: boolean;
  onCreateSuccess: () => void;
};

export function useSourceActions({
  queryClient,
  addToast,
  eventDrivenSyncEnabled,
  onCreateSuccess,
}: UseSourceActionsOptions) {
  const [activeSourceId, setActiveSourceId] = useState<string | null>(null);
  const [inlineAction, setInlineAction] = useState<InlineActionState>({
    phase: "idle",
    title: "",
    description: "",
  });

  const createMutation = useMutation({
    mutationFn: createSource,
    onMutate: async (input) => {
      setInlineAction({
        phase: "loading",
        title: "正在创建来源",
        description: `已提交 ${input.name}，等待 Core 确认。`,
      });
      await queryClient.cancelQueries({ queryKey: queryKeys.sources.all });
      await queryClient.cancelQueries({ queryKey: queryKeys.dashboard.systemStatus });

      const previousSources = queryClient.getQueryData<SourceListResponse>(
        queryKeys.sources.all,
      );
      const previousSystemStatus = queryClient.getQueryData<SystemStatusResponse>(
        queryKeys.dashboard.systemStatus,
      );

      const optimisticSourceId = `optimistic-source-${Date.now()}`;
      const now = new Date().toISOString();

      queryClient.setQueryData<SourceListResponse>(queryKeys.sources.all, (current) =>
        upsertSourceItem(current, {
          source: {
            id: optimisticSourceId,
            plugin_id: input.pluginId,
            name: input.name,
            status: "running",
            state_json: null,
            created_at: now,
            updated_at: now,
          },
          config: input.config,
        }),
      );

      queryClient.setQueryData<SystemStatusResponse | undefined>(
        queryKeys.dashboard.systemStatus,
        (current) =>
          patchSystemStatus(current, {
            activeSources: (current?.activeSources ?? 0) + 1,
          }),
      );

      return { previousSources, previousSystemStatus, optimisticSourceId };
    },
    onSuccess: (payload, _input, context) => {
      queryClient.setQueryData<SourceListResponse>(queryKeys.sources.all, (current) =>
        upsertSourceItem(
          removeSourceItem(current, context?.optimisticSourceId ?? ""),
          payload.source,
        ),
      );
      addToast({
        title: "来源创建成功",
        description: payload.source.source.name,
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "来源创建成功",
        description: `${payload.source.source.name} 已进入列表并完成本地同步。`,
      });
      onCreateSuccess();
    },
    onError: (error, _input, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.sources.all, context.previousSources);
        queryClient.setQueryData(
          queryKeys.dashboard.systemStatus,
          context.previousSystemStatus,
        );
      }
      addToast({
        title: "来源创建失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "来源创建失败",
        description: error instanceof Error ? error.message : "未知错误",
      });
    },
    onSettled: () => {
      if (eventDrivenSyncEnabled) {
        return;
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (input: { sourceId: string; name: string; config: Record<string, unknown> }) =>
      updateSource(input.sourceId, { name: input.name, config: input.config }),
    onMutate: async (input) => {
      setInlineAction({
        phase: "loading",
        title: "正在保存来源",
        description: `正在更新 ${input.name}。`,
      });
      await queryClient.cancelQueries({ queryKey: queryKeys.sources.all });
      const previousSources = queryClient.getQueryData<SourceListResponse>(
        queryKeys.sources.all,
      );

      queryClient.setQueryData<SourceListResponse | undefined>(
        queryKeys.sources.all,
        (current) =>
          patchSourceItem(current, input.sourceId, {
            name: input.name,
            config: input.config,
            updatedAt: new Date().toISOString(),
          }),
      );

      return { previousSources };
    },
    onSuccess: (payload) => {
      queryClient.setQueryData<SourceListResponse>(queryKeys.sources.all, (current) =>
        upsertSourceItem(current, payload.source),
      );
      addToast({
        title: "来源更新成功",
        description: payload.source.source.name,
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "来源保存完成",
        description: `${payload.source.source.name} 已同步到最新配置。`,
      });
    },
    onError: (error, _input, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.sources.all, context.previousSources);
      }
      addToast({
        title: "来源更新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "来源保存失败",
        description: error instanceof Error ? error.message : "未知错误",
      });
    },
    onSettled: () => {
      if (!eventDrivenSyncEnabled) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
        void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
      }
      setActiveSourceId(null);
    },
  });

  const refreshMutation = useMutation({
    mutationFn: (sourceId: string) => refreshSource(sourceId),
    onMutate: async (sourceId) => {
      setInlineAction({
        phase: "loading",
        title: "正在刷新来源",
        description: `来源 ${sourceId} 已进入刷新队列。`,
      });
      await queryClient.cancelQueries({ queryKey: queryKeys.sources.all });
      const previousSources = queryClient.getQueryData<SourceListResponse>(
        queryKeys.sources.all,
      );
      queryClient.setQueryData<SourceListResponse | undefined>(
        queryKeys.sources.all,
        (current) =>
          patchSourceItem(current, sourceId, {
            status: "running",
            updatedAt: new Date().toISOString(),
          }),
      );
      return { previousSources };
    },
    onSuccess: (payload) => {
      queryClient.setQueryData<SourceListResponse | undefined>(
        queryKeys.sources.all,
        (current) =>
          patchSourceItem(current, payload.source_id, {
            status: "healthy",
            updatedAt: new Date().toISOString(),
          }),
      );
      addToast({
        title: "来源刷新成功",
        description: `${payload.source_id} 返回 ${payload.node_count} 个节点`,
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "来源刷新完成",
        description: `${payload.source_id} 返回 ${payload.node_count} 个节点。`,
      });
    },
    onError: (error, _sourceId, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.sources.all, context.previousSources);
      }
      addToast({
        title: "来源刷新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "来源刷新失败",
        description: error instanceof Error ? error.message : "未知错误",
      });
    },
    onSettled: () => {
      if (!eventDrivenSyncEnabled) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
        void queryClient.invalidateQueries({ queryKey: queryKeys.runs.logsRoot });
        void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
        void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.logsRoot });
      }
      setActiveSourceId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (sourceId: string) => deleteSource(sourceId),
    onMutate: async (sourceId) => {
      setInlineAction({
        phase: "loading",
        title: "正在删除来源",
        description: `来源 ${sourceId} 删除请求已提交。`,
      });
      await queryClient.cancelQueries({ queryKey: queryKeys.sources.all });
      await queryClient.cancelQueries({ queryKey: queryKeys.dashboard.systemStatus });

      const previousSources = queryClient.getQueryData<SourceListResponse>(
        queryKeys.sources.all,
      );
      const previousSystemStatus = queryClient.getQueryData<SystemStatusResponse>(
        queryKeys.dashboard.systemStatus,
      );
      const removedSource = previousSources?.sources.find(
        (entry) => entry.source.id === sourceId,
      );

      queryClient.setQueryData<SourceListResponse | undefined>(
        queryKeys.sources.all,
        (current) => removeSourceItem(current, sourceId),
      );

      queryClient.setQueryData<SystemStatusResponse | undefined>(
        queryKeys.dashboard.systemStatus,
        (current) => {
          if (!current) {
            return current;
          }
          const decrement = removedSource?.source.status !== "disabled" ? 1 : 0;
          return patchSystemStatus(current, {
            activeSources: Math.max(0, current.activeSources - decrement),
          });
        },
      );

      return { previousSources, previousSystemStatus };
    },
    onSuccess: () => {
      addToast({
        title: "来源已删除",
        description: "来源记录及其关联缓存已清理。",
        variant: "warning",
      });
      setInlineAction({
        phase: "success",
        title: "来源删除成功",
        description: "来源记录及关联缓存已完成清理。",
      });
    },
    onError: (error, _sourceId, context) => {
      if (context) {
        queryClient.setQueryData(queryKeys.sources.all, context.previousSources);
        queryClient.setQueryData(
          queryKeys.dashboard.systemStatus,
          context.previousSystemStatus,
        );
      }
      addToast({
        title: "来源删除失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "来源删除失败",
        description: error instanceof Error ? error.message : "未知错误",
      });
    },
    onSettled: () => {
      if (!eventDrivenSyncEnabled) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
        void queryClient.invalidateQueries({ queryKey: queryKeys.runs.sources });
        void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
      }
      setActiveSourceId(null);
    },
  });

  return {
    activeSourceId,
    inlineAction,
    setActiveSourceId,
    createMutation,
    updateMutation,
    refreshMutation,
    deleteMutation,
  };
}
