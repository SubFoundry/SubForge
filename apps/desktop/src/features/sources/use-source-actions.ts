import { useMutation, type QueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { createSource, deleteSource, refreshSource, updateSource } from "../../lib/api";
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
  onCreateSuccess: () => void;
};

export function useSourceActions({
  queryClient,
  addToast,
  onCreateSuccess,
}: UseSourceActionsOptions) {
  const [activeSourceId, setActiveSourceId] = useState<string | null>(null);

  const createMutation = useMutation({
    mutationFn: createSource,
    onMutate: async (input) => {
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
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (input: { sourceId: string; name: string; config: Record<string, unknown> }) =>
      updateSource(input.sourceId, { name: input.name, config: input.config }),
    onMutate: async (input) => {
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
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
      setActiveSourceId(null);
    },
  });

  const refreshMutation = useMutation({
    mutationFn: (sourceId: string) => refreshSource(sourceId),
    onMutate: async (sourceId) => {
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
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
      void queryClient.invalidateQueries({ queryKey: queryKeys.runs.logsRoot });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.logsRoot });
      setActiveSourceId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (sourceId: string) => deleteSource(sourceId),
    onMutate: async (sourceId) => {
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
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sources.all });
      void queryClient.invalidateQueries({ queryKey: queryKeys.runs.sources });
      void queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.systemStatus });
      setActiveSourceId(null);
    },
  });

  return {
    activeSourceId,
    setActiveSourceId,
    createMutation,
    updateMutation,
    refreshMutation,
    deleteMutation,
  };
}
