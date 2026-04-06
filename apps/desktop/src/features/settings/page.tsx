import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import {
  desktopGetAutostart,
  desktopSetAutostart,
  updateSystemSettings,
} from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { WindowCloseBehavior } from "../../types/core";
import { CLOSE_BEHAVIOR_OPTIONS } from "./constants";
import {
  InlineActionFeedback,
  type InlineActionState,
} from "../../components/inline-action-feedback";

export default function SettingsPage() {
  const theme = useCoreUiStore((state) => state.theme);
  const setTheme = useCoreUiStore((state) => state.setTheme);
  const idleAutoCloseMinutes = useCoreUiStore((state) => state.idleAutoCloseMinutes);
  const autostartEnabled = useCoreUiStore((state) => state.autostartEnabled);
  const setAutostartEnabled = useCoreUiStore((state) => state.setAutostartEnabled);
  const setIdleAutoCloseMinutes = useCoreUiStore(
    (state) => state.setIdleAutoCloseMinutes,
  );
  const windowCloseBehavior = useCoreUiStore((state) => state.windowCloseBehavior);
  const setWindowCloseBehavior = useCoreUiStore(
    (state) => state.setWindowCloseBehavior,
  );
  const status = useCoreUiStore((state) => state.status);
  const addToast = useCoreUiStore((state) => state.addToast);
  const [idleMinutesInput, setIdleMinutesInput] = useState(
    String(idleAutoCloseMinutes),
  );
  const [closeBehaviorInput, setCloseBehaviorInput] = useState<WindowCloseBehavior>(
    windowCloseBehavior,
  );
  const [inlineAction, setInlineAction] = useState<InlineActionState>({
    phase: "idle",
    title: "",
    description: "",
  });

  useEffect(() => {
    setIdleMinutesInput(String(idleAutoCloseMinutes));
  }, [idleAutoCloseMinutes]);

  useEffect(() => {
    setCloseBehaviorInput(windowCloseBehavior);
  }, [windowCloseBehavior]);

  const updateThemeMutation = useMutation({
    mutationFn: (nextTheme: "dark" | "light") =>
      updateSystemSettings({ theme: nextTheme }),
    onMutate: (nextTheme) => {
      setInlineAction({
        phase: "loading",
        title: "正在保存主题",
        description: `正在切换到${nextTheme === "dark" ? "深色" : "浅色"}主题。`,
      });
    },
    onSuccess: (_, nextTheme) => {
      setTheme(nextTheme);
      addToast({
        title: "主题已更新",
        description: `已持久化为 ${nextTheme === "dark" ? "深色" : "浅色"} 主题。`,
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "主题保存成功",
        description: `当前主题：${nextTheme === "dark" ? "深色" : "浅色"}。`,
      });
    },
    onError: (error) => {
      addToast({
        title: "主题保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "主题保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
      });
    },
  });

  const autostartMutation = useMutation({
    mutationFn: (enabled: boolean) => desktopSetAutostart(enabled),
    onMutate: (enabled) => {
      setInlineAction({
        phase: "loading",
        title: "正在更新开机自启",
        description: enabled ? "启用中..." : "关闭中...",
      });
    },
    onSuccess: (enabled) => {
      setAutostartEnabled(enabled);
      addToast({
        title: enabled ? "开机自启已启用" : "开机自启已关闭",
        description: enabled
          ? "下次系统启动后会自动启动 SubForge Desktop。"
          : "系统启动时将不再自动启动 SubForge Desktop。",
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "开机自启已更新",
        description: enabled ? "当前为启用状态。" : "当前为关闭状态。",
      });
    },
    onError: (error) => {
      addToast({
        title: "开机自启设置失败",
        description: error instanceof Error ? error.message : "调用系统开机自启接口失败。",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "开机自启设置失败",
        description: error instanceof Error ? error.message : "调用系统开机自启接口失败。",
      });
    },
  });

  const refreshAutostartMutation = useMutation({
    mutationFn: desktopGetAutostart,
    onMutate: () => {
      setInlineAction({
        phase: "loading",
        title: "正在读取开机自启状态",
        description: "请稍候...",
      });
    },
    onSuccess: (enabled) => {
      setAutostartEnabled(enabled);
      addToast({
        title: "开机自启状态已刷新",
        description: enabled ? "当前为启用状态。" : "当前为关闭状态。",
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "开机自启状态已刷新",
        description: enabled ? "当前为启用状态。" : "当前为关闭状态。",
      });
    },
    onError: (error) => {
      addToast({
        title: "开机自启状态读取失败",
        description: error instanceof Error ? error.message : "读取失败，请稍后重试。",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "读取开机自启状态失败",
        description: error instanceof Error ? error.message : "读取失败，请稍后重试。",
      });
    },
  });

  const updateGuiLifecycleMutation = useMutation({
    mutationFn: ({
      nextIdleMinutes,
      nextCloseBehavior,
    }: {
      nextIdleMinutes: number;
      nextCloseBehavior: WindowCloseBehavior;
    }) =>
      updateSystemSettings({
        gui_idle_auto_close_minutes: String(nextIdleMinutes),
        gui_close_behavior: nextCloseBehavior,
        tray_minimize: nextCloseBehavior === "tray_minimize" ? "true" : "false",
      }),
    onMutate: (payload) => {
      setInlineAction({
        phase: "loading",
        title: "正在保存 GUI 行为",
        description: `空闲关闭 ${payload.nextIdleMinutes} 分钟，关闭行为 ${payload.nextCloseBehavior}。`,
      });
    },
    onSuccess: (_, payload) => {
      setIdleAutoCloseMinutes(payload.nextIdleMinutes);
      setWindowCloseBehavior(payload.nextCloseBehavior);
      addToast({
        title: "GUI 行为设置已保存",
        description: "空闲自动关闭和窗口关闭行为已更新。",
        variant: "default",
      });
      setInlineAction({
        phase: "success",
        title: "GUI 行为设置已保存",
        description: "空闲自动关闭与关闭窗口行为已同步。",
      });
    },
    onError: (error) => {
      addToast({
        title: "GUI 行为设置保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
        variant: "error",
      });
      setInlineAction({
        phase: "error",
        title: "GUI 行为设置保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
      });
    },
  });

  const parsedIdleMinutes = useMemo(() => {
    const parsed = Number.parseInt(idleMinutesInput, 10);
    if (!Number.isFinite(parsed)) {
      return null;
    }
    if (parsed < 0 || parsed > 10_080) {
      return null;
    }
    return parsed;
  }, [idleMinutesInput]);

  const idleMinutesValidationMessage =
    parsedIdleMinutes === null ? "请输入 0 到 10080 的整数分钟数。" : null;

  const isLifecycleDirty =
    parsedIdleMinutes !== null &&
    (parsedIdleMinutes !== idleAutoCloseMinutes ||
      closeBehaviorInput !== windowCloseBehavior);

  const applyTheme = (nextTheme: "dark" | "light") => {
    if (!status?.running) {
      setTheme(nextTheme);
      addToast({
        title: "仅本地切换",
        description: "Core 当前离线，主题已本地生效，待恢复后请重新保存。",
        variant: "warning",
      });
      return;
    }

    updateThemeMutation.mutate(nextTheme);
  };

  const saveGuiLifecycleSettings = () => {
    if (parsedIdleMinutes === null) {
      addToast({
        title: "参数不合法",
        description: "空闲关闭分钟数必须是 0 到 10080 的整数。",
        variant: "warning",
      });
      return;
    }

    if (!status?.running) {
      setIdleAutoCloseMinutes(parsedIdleMinutes);
      setWindowCloseBehavior(closeBehaviorInput);
      addToast({
        title: "仅本地更新",
        description: "Core 当前离线，设置已本地生效，待恢复后请重新保存。",
        variant: "warning",
      });
      return;
    }

    updateGuiLifecycleMutation.mutate({
      nextIdleMinutes: parsedIdleMinutes,
      nextCloseBehavior: closeBehaviorInput,
    });
  };

  return (
    <section className="ui-page">
      <header className="ui-page-header">
        <div>
          <h2 className="ui-page-title">Settings</h2>
          <p className="ui-page-desc">配置主题、空闲自动关闭与窗口关闭行为。</p>
        </div>
        <span className="ui-badge ui-badge-muted">Desktop Runtime</span>
      </header>
      <InlineActionFeedback state={inlineAction} />

      <article className="ui-card">
        <h3 className="ui-card-title">外观主题</h3>
        <p className="ui-card-desc">
          通过 `/api/system/settings` 持久化 `theme`，重启后保持一致。
        </p>
        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() => applyTheme("dark")}
            className={`ui-btn ui-focus ${
              theme === "dark"
                ? "ui-btn-primary"
                : "ui-btn-secondary"
            }`}
            disabled={updateThemeMutation.isPending}
          >
            深色
          </button>
          <button
            type="button"
            onClick={() => applyTheme("light")}
            className={`ui-btn ui-focus ${
              theme === "light"
                ? "ui-btn-primary"
                : "ui-btn-secondary"
            }`}
            disabled={updateThemeMutation.isPending}
          >
            浅色
          </button>
        </div>
      </article>

      <article className="ui-card">
        <h3 className="ui-card-title">系统集成</h3>
        <p className="ui-card-desc">
          配置 SubForge Desktop 是否随系统启动自动运行。
        </p>

        <div className="mt-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            className={`ui-btn ui-focus ${autostartEnabled ? "ui-btn-primary" : "ui-btn-secondary"}`}
            disabled={autostartMutation.isPending}
            onClick={() => autostartMutation.mutate(!autostartEnabled)}
          >
            {autostartMutation.isPending
              ? "应用中..."
              : autostartEnabled
                ? "关闭开机自启"
                : "启用开机自启"}
          </button>

          <button
            type="button"
            className="ui-btn ui-btn-secondary ui-focus"
            disabled={refreshAutostartMutation.isPending}
            onClick={() => refreshAutostartMutation.mutate()}
          >
            {refreshAutostartMutation.isPending ? "刷新中..." : "刷新状态"}
          </button>

          <span className="text-xs text-[var(--muted-text)]">
            当前：{autostartEnabled ? "已启用" : "未启用"}
          </span>
        </div>
      </article>

      <article className="ui-card">
        <h3 className="ui-card-title">GUI 行为</h3>
        <p className="ui-card-desc">
          设置空闲自动关闭与窗口关闭行为，均通过 `/api/system/settings` 持久化。
        </p>

        <div className="mt-4 space-y-3">
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-[var(--app-text)]">空闲 N 分钟后自动关闭 GUI（0=禁用）</span>
            <input
              type="number"
              min={0}
              max={10080}
              step={1}
              value={idleMinutesInput}
              onChange={(event) => setIdleMinutesInput(event.target.value)}
              className="ui-input ui-focus w-full max-w-xs"
            />
          </label>
          {idleMinutesValidationMessage ? (
            <p className="text-xs text-[var(--warn-text)]">{idleMinutesValidationMessage}</p>
          ) : null}
        </div>

        <div className="mt-5 space-y-3">
          <p className="text-sm text-[var(--app-text)]">关闭窗口行为</p>
          <div className="space-y-2">
            {CLOSE_BEHAVIOR_OPTIONS.map((option) => (
              <label
                key={option.value}
                className="flex cursor-pointer items-start gap-3 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2"
              >
                <input
                  type="radio"
                  name="window-close-behavior"
                  value={option.value}
                  checked={closeBehaviorInput === option.value}
                  onChange={() => setCloseBehaviorInput(option.value)}
                  className="ui-focus mt-1"
                />
                <span className="space-y-1">
                  <span className="block text-sm font-medium text-[var(--app-text)]">
                    {option.label}
                  </span>
                  <span className="block text-xs text-[var(--muted-text)]">
                    {option.description}
                  </span>
                </span>
              </label>
            ))}
          </div>
        </div>

        <div className="mt-5">
          <button
            type="button"
            onClick={saveGuiLifecycleSettings}
            disabled={
              updateGuiLifecycleMutation.isPending ||
              parsedIdleMinutes === null ||
              !isLifecycleDirty
            }
            className="ui-btn ui-btn-primary ui-focus"
          >
            保存 GUI 行为设置
          </button>
        </div>
      </article>
    </section>
  );
}
