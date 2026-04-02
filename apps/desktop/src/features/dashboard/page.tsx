import { Skeleton } from "../../components/skeleton";
import { useCoreUiStore } from "../../stores/core-ui-store";

function StatusCard({
  title,
  value,
  hint,
}: {
  title: string;
  value: string;
  hint: string;
}) {
  return (
    <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/50 p-4">
      <p className="text-xs uppercase tracking-wide text-[var(--muted-text)]">{title}</p>
      <p className="mt-2 text-xl font-semibold text-[var(--app-text)]">{value}</p>
      <p className="mt-1 text-xs text-[var(--muted-text)]">{hint}</p>
    </article>
  );
}

export default function DashboardPage() {
  const phase = useCoreUiStore((state) => state.phase);
  const status = useCoreUiStore((state) => state.status);
  const heartbeatAt = useCoreUiStore((state) => state.heartbeatAt);
  const lastEvent = useCoreUiStore((state) => state.lastEvent);
  const eventStreamActive = useCoreUiStore((state) => state.eventStreamActive);
  const error = useCoreUiStore((state) => state.error);

  const isBooting = phase === "booting" || (phase === "idle" && !status);

  return (
    <section className="space-y-5">
      <header>
        <h2 className="text-2xl font-semibold">Dashboard</h2>
        <p className="mt-1 text-sm text-[var(--muted-text)]">
          Core 连接状态、心跳与事件流概览
        </p>
      </header>

      {isBooting ? (
        <div className="grid gap-3 md:grid-cols-3">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : (
        <div className="grid gap-3 md:grid-cols-3">
          <StatusCard
            title="Core 状态"
            value={status?.running ? "运行中" : "未运行"}
            hint={status?.baseUrl ?? "-"}
          />
          <StatusCard
            title="事件流"
            value={eventStreamActive ? "已连接" : "未连接"}
            hint={lastEvent?.event ?? "暂无事件"}
          />
          <StatusCard
            title="心跳"
            value={heartbeatAt ?? "未建立"}
            hint={error ?? "最近一次轮询成功"}
          />
        </div>
      )}

      <article className="rounded-xl border border-[var(--panel-border)] bg-[var(--panel-muted)]/45 p-4">
        <h3 className="text-sm font-semibold text-[var(--app-text)]">最近事件</h3>
        {lastEvent ? (
          <div className="mt-3 space-y-1 text-sm">
            <p className="font-medium text-[var(--accent-strong)]">{lastEvent.event}</p>
            <p className="text-[var(--app-text)]">{lastEvent.message}</p>
            <p className="text-xs text-[var(--muted-text)]">
              source: {lastEvent.sourceId ?? "-"} | timestamp:{" "}
              {lastEvent.timestamp ?? "-"}
            </p>
          </div>
        ) : (
          <p className="mt-3 text-sm text-[var(--muted-text)]">暂无事件。</p>
        )}
      </article>
    </section>
  );
}
