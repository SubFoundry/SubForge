import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { updateSystemSettings } from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { WindowCloseBehavior } from "../../types/core";

const CLOSE_BEHAVIOR_OPTIONS: Array<{
  value: WindowCloseBehavior;
  label: string;
  description: string;
}> = [
  {
    value: "tray_minimize",
    label: "最小化到托盘",
    description: "点击窗口关闭按钮时仅隐藏窗口，GUI 进程保持运行。",
  },
  {
    value: "close_gui",
    label: "仅关闭 GUI",
    description: "关闭管理界面进程，Core 守护进程继续运行。",
  },
  {
    value: "close_gui_and_stop_core",
    label: "关闭 GUI 并停止 Core",
    description: "关闭管理界面时同时停止 Core 进程。",
  },
];

export default function SettingsPage() {
  const theme = useCoreUiStore((state) => state.theme);
  const setTheme = useCoreUiStore((state) => state.setTheme);
  const idleAutoCloseMinutes = useCoreUiStore((state) => state.idleAutoCloseMinutes);
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

  useEffect(() => {
    setIdleMinutesInput(String(idleAutoCloseMinutes));
  }, [idleAutoCloseMinutes]);

  useEffect(() => {
    setCloseBehaviorInput(windowCloseBehavior);
  }, [windowCloseBehavior]);

  const updateThemeMutation = useMutation({
    mutationFn: (nextTheme: "dark" | "light") =>
      updateSystemSettings({ theme: nextTheme }),
    onSuccess: (_, nextTheme) => {
      setTheme(nextTheme);
      addToast({
        title: "主题已更新",
        description: `已持久化为 ${nextTheme === "dark" ? "深色" : "浅色"} 主题。`,
        variant: "default",
      });
    },
    onError: (error) => {
      addToast({
        title: "主题保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
        variant: "error",
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
    onSuccess: (_, payload) => {
      setIdleAutoCloseMinutes(payload.nextIdleMinutes);
      setWindowCloseBehavior(payload.nextCloseBehavior);
      addToast({
        title: "GUI 行为设置已保存",
        description: "空闲自动关闭和窗口关闭行为已更新。",
        variant: "default",
      });
    },
    onError: (error) => {
      addToast({
        title: "GUI 行为设置保存失败",
        description: error instanceof Error ? error.message : "Core 设置接口调用失败。",
        variant: "error",
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
    <section className="space-y-5">
      <header>
        <h2 className="text-2xl font-semibold">Settings</h2>
        <p className="mt-1 text-sm text-[var(--muted-text)]">
          配置主题、空闲自动关闭与窗口关闭行为。
        </p>
      </header>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/55 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">外观主题</h3>
        <p className="mt-1 text-xs text-[var(--muted-text)]">
          通过 `/api/system/settings` 持久化 `theme`，重启后保持一致。
        </p>
        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() => applyTheme("dark")}
            className={`rounded-lg px-3 py-2 text-sm font-medium transition ${
              theme === "dark"
                ? "bg-[var(--accent-soft)] text-[var(--accent-strong)]"
                : "bg-[var(--panel-bg)] text-[var(--muted-text)] hover:bg-[var(--panel-muted)]"
            }`}
            disabled={updateThemeMutation.isPending}
          >
            深色
          </button>
          <button
            type="button"
            onClick={() => applyTheme("light")}
            className={`rounded-lg px-3 py-2 text-sm font-medium transition ${
              theme === "light"
                ? "bg-[var(--accent-soft)] text-[var(--accent-strong)]"
                : "bg-[var(--panel-bg)] text-[var(--muted-text)] hover:bg-[var(--panel-muted)]"
            }`}
            disabled={updateThemeMutation.isPending}
          >
            浅色
          </button>
        </div>
      </article>

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/55 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">GUI 行为</h3>
        <p className="mt-1 text-xs text-[var(--muted-text)]">
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
              className="w-full max-w-xs rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)] outline-none focus:border-[var(--accent-strong)]"
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
                  className="mt-1"
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
            className="rounded-lg bg-[var(--accent-soft)] px-3 py-2 text-sm font-medium text-[var(--accent-strong)] transition hover:brightness-105 disabled:cursor-not-allowed disabled:opacity-50"
          >
            保存 GUI 行为设置
          </button>
        </div>
      </article>
    </section>
  );
}
