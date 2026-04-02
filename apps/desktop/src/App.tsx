import { Link, Outlet, useLocation } from "react-router-dom";
import { StatusIndicator } from "./components/status-indicator";
import { useCoreUiStore } from "./stores/core-ui-store";

const navItems = [
  { path: "/", label: "Dashboard" },
  { path: "/plugins", label: "Plugins" },
  { path: "/sources", label: "Sources" },
  { path: "/profiles", label: "Profiles" },
  { path: "/runs", label: "Runs" },
  { path: "/settings", label: "Settings" },
];

export default function App() {
  const location = useLocation();
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const heartbeatAt = useCoreUiStore((state) => state.heartbeatAt);
  const error = useCoreUiStore((state) => state.error);

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
        <div className="border-b border-rose-300/30 bg-rose-900/35 px-4 py-2 text-xs text-rose-50">
          {phase === "error" ? error ?? "Core 连接异常" : "Core 未运行，正在等待重连。"}
        </div>
      )}

      <div className="mx-auto grid min-h-screen max-w-[1380px] grid-cols-1 gap-4 p-4 md:grid-cols-[220px_1fr] md:gap-6 md:p-6">
        <aside className="rounded-2xl border border-[var(--panel-border)] bg-[var(--panel-bg)] p-4 shadow-[0_16px_60px_rgba(0,0,0,0.35)] backdrop-blur">
          <h1 className="mb-4 text-xl font-semibold tracking-wide text-[var(--accent-strong)]">
            SubForge
          </h1>
          <nav className="flex gap-2 overflow-x-auto pb-1 md:block md:space-y-2">
            {navItems.map((item) => {
              const active =
                item.path === "/"
                  ? location.pathname === "/"
                  : location.pathname.startsWith(item.path);
              return (
                <Link
                  key={item.path}
                  to={item.path}
                  className={`block shrink-0 rounded-lg px-3 py-2 text-sm transition ${
                    active
                      ? "bg-[var(--accent-soft)] text-[var(--accent-strong)]"
                      : "text-[var(--muted-text)] hover:bg-[var(--panel-muted)]"
                  }`}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>
        </aside>

        <main className="rounded-2xl border border-[var(--panel-border)] bg-[var(--panel-bg)] p-4 backdrop-blur md:p-6">
          <header className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--panel-border)] pb-4">
            <StatusIndicator status={indicator.status} label={indicator.label} />
            <div className="text-xs text-[var(--muted-text)]">
              <span>版本：{status?.version ?? "-"}</span>
              <span className="mx-2">|</span>
              <span>心跳：{heartbeatAt ?? "未建立"}</span>
            </div>
          </header>
          <Outlet />
        </main>
      </div>
    </div>
  );
}
