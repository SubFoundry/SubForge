import { Link, Outlet, useLocation } from "react-router-dom";
import { StatusIndicator } from "./components/status-indicator";
import { useCoreUiStore } from "./stores/core-ui-store";

const navItems = [
  { path: "/", label: "总览", hint: "Dashboard", tag: "DB" },
  { path: "/plugins", label: "插件", hint: "Plugin Center", tag: "PL" },
  { path: "/sources", label: "来源", hint: "Sources", tag: "SC" },
  { path: "/profiles", label: "配置", hint: "Profiles", tag: "PF" },
  { path: "/runs", label: "记录", hint: "Runs", tag: "RN" },
  { path: "/settings", label: "设置", hint: "Settings", tag: "ST" },
];

export default function App() {
  const location = useLocation();
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const heartbeatAt = useCoreUiStore((state) => state.heartbeatAt);
  const error = useCoreUiStore((state) => state.error);
  const currentNavItem =
    navItems.find((item) =>
      item.path === "/" ? location.pathname === "/" : location.pathname.startsWith(item.path),
    ) ?? navItems[0];

  const indicator =
    phase === "running"
      ? eventStreamActive
        ? { status: "online" as const, label: "Core 在线" }
        : { status: "degraded" as const, label: "Core 在线（事件流重连中）" }
      : phase === "booting"
        ? { status: "degraded" as const, label: "Core 启动中" }
        : { status: "offline" as const, label: "Core 未连接" };

  return (
    <div className="min-h-screen bg-[var(--app-bg)] text-[var(--app-text)]">
      {(phase === "disconnected" || phase === "error") && (
        <div className="border-b border-rose-300/30 bg-rose-900/35 px-4 py-2 text-xs text-rose-50 backdrop-blur">
          {phase === "error" ? error ?? "Core 连接异常" : "Core 未运行，正在等待重连。"}
        </div>
      )}

      <div className="mx-auto flex min-h-screen max-w-[1540px] flex-col gap-4 p-4 md:gap-6 md:p-6 xl:flex-row">
        <aside className="w-full rounded-2xl border border-[var(--panel-border)] bg-[var(--panel-bg)] p-4 shadow-[0_24px_70px_rgba(5,12,24,0.45)] backdrop-blur xl:sticky xl:top-6 xl:h-[calc(100vh-3rem)] xl:w-[282px] xl:min-w-[282px] xl:overflow-y-auto">
          <div className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/72 px-4 py-3">
            <p className="text-xs uppercase tracking-[0.2em] text-[var(--muted-text)]">
              SubForge
            </p>
            <h1 className="mt-1 text-2xl font-semibold text-[var(--app-text)]">Control Deck</h1>
            <p className="mt-1 text-xs text-[var(--muted-text)]">Core + Desktop 管理中枢</p>
          </div>

          <nav className="mt-4 flex gap-2 overflow-x-auto pb-1 md:block md:space-y-2">
            {navItems.map((item) => {
              const active =
                item.path === "/"
                  ? location.pathname === "/"
                  : location.pathname.startsWith(item.path);
              return (
                <Link
                  key={item.path}
                  to={item.path}
                  aria-current={active ? "page" : undefined}
                  className={`ui-focus group flex shrink-0 items-center gap-3 rounded-lg border px-3 py-2 text-sm transition ${
                    active
                      ? "border-[var(--accent-border)] bg-[var(--accent-soft)] text-[var(--app-text)]"
                      : "border-transparent text-[var(--muted-text)] hover:border-[var(--panel-border)] hover:bg-[var(--panel-muted)]"
                  }`}
                >
                  <span
                    className={`inline-flex h-8 w-8 items-center justify-center rounded-md border text-[10px] font-semibold tracking-wide ${
                      active
                        ? "border-[var(--accent-border)] bg-[var(--accent-chip-bg)] text-[var(--accent-strong)]"
                        : "border-[var(--panel-border)] bg-[var(--panel-bg)] text-[var(--muted-text)]"
                    }`}
                  >
                    {item.tag}
                  </span>
                  <span className="space-y-0.5">
                    <span className="block text-sm font-medium tracking-wide">{item.label}</span>
                    <span className="block text-[11px] text-[var(--muted-text)]">{item.hint}</span>
                  </span>
                </Link>
              );
            })}
          </nav>

          <div className="mt-4 rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/35 px-3 py-3 text-xs">
            <p className="text-[var(--muted-text)]">连接状态</p>
            <p className="mt-1 text-sm font-medium text-[var(--app-text)]">{indicator.label}</p>
            <p className="mt-2 text-[var(--muted-text)]">心跳</p>
            <p className="mt-1 text-sm text-[var(--app-text)]">{heartbeatAt ?? "未建立"}</p>
          </div>
        </aside>

        <section className="flex w-full min-w-0 flex-1 flex-col gap-4">
          <header className="rounded-2xl border border-[var(--panel-border)] bg-[var(--panel-bg)] px-4 py-3 shadow-[0_16px_40px_rgba(5,12,24,0.32)] backdrop-blur md:px-6">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <p className="text-xs uppercase tracking-[0.16em] text-[var(--muted-text)]">
                  {currentNavItem.hint}
                </p>
                <h2 className="mt-1 text-2xl font-semibold text-[var(--app-text)]">
                  {currentNavItem.label}
                </h2>
              </div>
              <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--muted-text)]">
                <StatusIndicator status={indicator.status} label={indicator.label} />
                <span className="rounded-md border border-[var(--panel-border)] bg-[var(--panel-muted)] px-2 py-1">
                  版本 {status?.version ?? "-"}
                </span>
                <span className="rounded-md border border-[var(--panel-border)] bg-[var(--panel-muted)] px-2 py-1">
                  心跳 {heartbeatAt ?? "未建立"}
                </span>
              </div>
            </div>
          </header>

          <main className="flex-1 rounded-2xl border border-[var(--panel-border)] bg-[var(--panel-bg)] p-4 shadow-[0_22px_55px_rgba(5,12,24,0.28)] backdrop-blur md:p-6">
            <div className="h-full overflow-y-auto pr-1">
              <Outlet />
            </div>
          </main>
        </section>
      </div>
    </div>
  );
}
