import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { coreStart, coreStatus, fetchCoreHealth } from "../../lib/api";
import type { CoreStatus } from "../../types/core";
import { useCoreUiStore } from "../../stores/core-ui-store";

export default function DashboardPage() {
  const [status, setStatus] = useState<CoreStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const setLoading = useCoreUiStore((state) => state.setLoading);

  useEffect(() => {
    let active = true;

    async function bootstrap() {
      setLoading(true);
      try {
        const current = await coreStatus();
        if (!active) {
          return;
        }

        if (!current.running) {
          const started = await coreStart();
          if (active) {
            setStatus(started);
          }
        } else {
          setStatus(current);
        }
      } catch (err) {
        if (active) {
          setError(err instanceof Error ? err.message : "Core 启动失败");
        }
      } finally {
        if (active) {
          setLoading(false);
        }
      }
    }

    void bootstrap();

    return () => {
      active = false;
    };
  }, [setLoading]);

  const healthQuery = useQuery({
    queryKey: ["core-health", status?.running],
    queryFn: fetchCoreHealth,
    enabled: Boolean(status?.running),
    staleTime: 5_000,
  });

  return (
    <section className="space-y-4">
      <header>
        <h2 className="text-2xl font-semibold">Dashboard</h2>
        <p className="mt-1 text-sm text-slate-300">P1 联通验证面板</p>
      </header>

      <div className="rounded-xl border border-slate-700 bg-slate-900/60 p-4">
        <p className="text-sm text-slate-300">Core 状态</p>
        <p className="mt-2 text-lg font-medium">
          {error
            ? `异常：${error}`
            : status?.running
              ? "运行中"
              : "未运行"}
        </p>
        <p className="mt-1 text-xs text-slate-400">Base URL: {status?.baseUrl ?? "-"}</p>
        <p className="mt-1 text-xs text-slate-400">
          版本: {healthQuery.data?.version ?? status?.version ?? "-"}
        </p>
      </div>
    </section>
  );
}