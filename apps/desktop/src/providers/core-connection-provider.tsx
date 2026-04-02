import { listen } from "@tauri-apps/api/event";
import { type PropsWithChildren, useEffect, useRef } from "react";
import {
  coreEventsStart,
  coreStart,
  coreStatus,
  fetchCoreHealth,
  fetchSystemSettings,
} from "../lib/api";
import { useCoreUiStore } from "../stores/core-ui-store";
import type { CoreBridgeEvent } from "../types/core";

const HEARTBEAT_INTERVAL_MS = 10_000;

export function CoreConnectionProvider({ children }: PropsWithChildren) {
  const setPhase = useCoreUiStore((state) => state.setPhase);
  const setStatus = useCoreUiStore((state) => state.setStatus);
  const setError = useCoreUiStore((state) => state.setError);
  const setHeartbeatAt = useCoreUiStore((state) => state.setHeartbeatAt);
  const setEventStreamActive = useCoreUiStore((state) => state.setEventStreamActive);
  const setLastEvent = useCoreUiStore((state) => state.setLastEvent);
  const setTheme = useCoreUiStore((state) => state.setTheme);
  const theme = useCoreUiStore((state) => state.theme);
  const addToast = useCoreUiStore((state) => state.addToast);
  const previousRunning = useRef<boolean>(false);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      setPhase("booting");
      try {
        let status = await coreStatus();
        if (!status.running) {
          status = await coreStart();
        }
        if (cancelled) {
          return;
        }

        setStatus(status);
        previousRunning.current = status.running;

        if (status.running) {
          setPhase("running");
          setError(null);
          await coreEventsStart();
          const health = await fetchCoreHealth();
          if (cancelled) {
            return;
          }
          setStatus({ ...status, version: health.version });

          try {
            const settings = await fetchSystemSettings();
            if (!cancelled && settings.settings.theme) {
              setTheme(settings.settings.theme === "light" ? "light" : "dark");
            }
          } catch {
            addToast({
              title: "设置读取失败",
              description: "已使用本地默认主题，稍后可在设置页重试。",
              variant: "warning",
            });
          }
        } else {
          setPhase("disconnected");
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        setPhase("error");
        setError(error instanceof Error ? error.message : "Core 启动失败");
        addToast({
          title: "Core 启动失败",
          description: "请检查 Core 进程与日志输出后重试。",
          variant: "error",
        });
      }
    }

    void bootstrap();

    return () => {
      cancelled = true;
    };
  }, [addToast, setError, setPhase, setStatus, setTheme]);

  useEffect(() => {
    let cancelled = false;

    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const status = await coreStatus();
          if (cancelled) {
            return;
          }
          setStatus(status);
          setHeartbeatAt(new Date().toISOString());

          if (status.running) {
            if (!previousRunning.current) {
              addToast({
                title: "Core 已重连",
                description: "管理连接恢复，可继续操作。",
                variant: "default",
              });
              await coreEventsStart();
            }
            const health = await fetchCoreHealth();
            if (!cancelled) {
              setStatus({ ...status, version: health.version });
              setPhase("running");
              setError(null);
            }
          } else {
            setPhase("disconnected");
            setEventStreamActive(false);
            setError("Core 未运行");
          }
          previousRunning.current = status.running;
        } catch (error) {
          if (cancelled) {
            return;
          }
          setPhase("error");
          setEventStreamActive(false);
          setError(error instanceof Error ? error.message : "Core 心跳失败");
        }
      })();
    }, HEARTBEAT_INTERVAL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [addToast, setError, setEventStreamActive, setHeartbeatAt, setPhase, setStatus]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;

    void listen<CoreBridgeEvent>("core://event", (event) => {
      if (!active) {
        return;
      }
      const payload = event.payload;
      if (payload.kind === "connected") {
        setEventStreamActive(true);
        return;
      }
      if (payload.kind === "disconnected") {
        setEventStreamActive(false);
        return;
      }
      if (payload.kind === "error") {
        setEventStreamActive(false);
        if (payload.message) {
          setError(payload.message);
        }
        return;
      }
      if (payload.kind === "event" && payload.payload) {
        setLastEvent(payload.payload);
        if (
          payload.payload.event === "refresh:error" ||
          payload.payload.event === "source:degraded"
        ) {
          addToast({
            title: payload.payload.event,
            description: payload.payload.message,
            variant: "warning",
          });
        }
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      active = false;
      if (unlisten) {
        unlisten();
      }
    };
  }, [addToast, setError, setEventStreamActive, setLastEvent]);

  return children;
}
