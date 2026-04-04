import { useMutation, type QueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { createSource, deleteSource, refreshSource, updateSource } from "../../lib/api";
import type { ToastMessage } from "../../stores/core-ui-store";

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
    onSuccess: (payload) => {
      addToast({
        title: "来源创建成功",
        description: payload.source.source.name,
        variant: "default",
      });
      onCreateSuccess();
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源创建失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (input: { sourceId: string; name: string; config: Record<string, unknown> }) =>
      updateSource(input.sourceId, { name: input.name, config: input.config }),
    onSuccess: (payload) => {
      addToast({
        title: "来源更新成功",
        description: payload.source.source.name,
        variant: "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
    },
    onError: (error) => {
      addToast({
        title: "来源更新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveSourceId(null);
    },
  });

  const refreshMutation = useMutation({
    mutationFn: (sourceId: string) => refreshSource(sourceId),
    onSuccess: (payload) => {
      addToast({
        title: "来源刷新成功",
        description: `${payload.source_id} 返回 ${payload.node_count} 个节点`,
        variant: "default",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["runs", "logs"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源刷新失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
      setActiveSourceId(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (sourceId: string) => deleteSource(sourceId),
    onSuccess: () => {
      addToast({
        title: "来源已删除",
        description: "来源记录及其关联缓存已清理。",
        variant: "warning",
      });
      void queryClient.invalidateQueries({ queryKey: ["sources"] });
      void queryClient.invalidateQueries({ queryKey: ["runs", "sources"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-system-status"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard-logs"] });
    },
    onError: (error) => {
      addToast({
        title: "来源删除失败",
        description: error instanceof Error ? error.message : "未知错误",
        variant: "error",
      });
    },
    onSettled: () => {
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
