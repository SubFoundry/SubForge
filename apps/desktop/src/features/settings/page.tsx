import { useMutation } from "@tanstack/react-query";
import { updateSystemSettings } from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";

export default function SettingsPage() {
  const theme = useCoreUiStore((state) => state.theme);
  const setTheme = useCoreUiStore((state) => state.setTheme);
  const status = useCoreUiStore((state) => state.status);
  const addToast = useCoreUiStore((state) => state.addToast);

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

  return (
    <section className="space-y-5">
      <header>
        <h2 className="text-2xl font-semibold">Settings</h2>
        <p className="mt-1 text-sm text-[var(--muted-text)]">
          当前切片实现主题切换与 Core 持久化。
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
    </section>
  );
}
