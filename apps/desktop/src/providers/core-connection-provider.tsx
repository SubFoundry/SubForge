import { listen } from "@tauri-apps/api/event";
import { type PropsWithChildren, useEffect, useRef } from "react";
import {
  coreEventsStart,
  desktopGetAutostart,
  coreStart,
  desktopAutoCloseGui,
  coreStatus,
  fetchCoreHealth,
  fetchSystemSettings,
} from "../lib/api";
import { notifyDesktopForCoreEvent } from "../lib/desktop-notification";
import { useCoreUiStore } from "../stores/core-ui-store";
import type { CoreBridgeEvent, WindowCloseBehavior } from "../types/core";

const HEARTBEAT_INTERVAL_MS = 10_000;
const IDLE_CHECK_INTERVAL_MS = 15_000;
const DEFAULT_IDLE_AUTO_CLOSE_MINUTES = 30;

export function CoreConnectionProvider({ children }: PropsWithChildren) {
  const setPhase = useCoreUiStore((state) => state.setPhase);
  const setStatus = useCoreUiStore((state) => state.setStatus);
  const setError = useCoreUiStore((state) => state.setError);
  const setHeartbeatAt = useCoreUiStore((state) => state.setHeartbeatAt);
  const setEventStreamActive = useCoreUiStore((state) => state.setEventStreamActive);
  const pushEvent = useCoreUiStore((state) => state.pushEvent);
  const setLastRefreshAt = useCoreUiStore((state) => state.setLastRefreshAt);
  const setTheme = useCoreUiStore((state) => state.setTheme);
  const idleAutoCloseMinutes = useCoreUiStore((state) => state.idleAutoCloseMinutes);
  const setAutostartEnabled = useCoreUiStore((state) => state.setAutostartEnabled);
  const setIdleAutoCloseMinutes = useCoreUiStore(
    (state) => state.setIdleAutoCloseMinutes,
  );
  const setWindowCloseBehavior = useCoreUiStore(
    (state) => state.setWindowCloseBehavior,
  );
  const theme = useCoreUiStore((state) => state.theme);
  const addToast = useCoreUiStore((state) => state.addToast);
  const previousRunning = useRef<boolean>(false);
  const lastActivityAt = useRef<number>(Date.now());
  const idleCloseTriggered = useRef<boolean>(false);

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

        try {
          const autostartEnabled = await desktopGetAutostart();
          if (!cancelled) {
            setAutostartEnabled(autostartEnabled);
          }
        } catch {
          if (!cancelled) {
            addToast({
              title: "开机自启状态读取失败",
              description: "将使用本地默认值，可在设置页重试。",
              variant: "warning",
            });
          }
        }

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
            if (!cancelled) {
              if (settings.settings.theme) {
                setTheme(settings.settings.theme === "light" ? "light" : "dark");
              }
              setIdleAutoCloseMinutes(
                parseIdleAutoCloseMinutes(
                  settings.settings.gui_idle_auto_close_minutes,
                ),
              );
              setWindowCloseBehavior(
                parseWindowCloseBehavior(
                  settings.settings.gui_close_behavior,
                  settings.settings.tray_minimize,
                ),
              );
            }
          } catch {
            addToast({
              title: "设置读取失败",
              description: "已使用本地默认设置，稍后可在设置页重试。",
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
  }, [
    addToast,
    setAutostartEnabled,
    setError,
    setIdleAutoCloseMinutes,
    setPhase,
    setStatus,
    setTheme,
    setWindowCloseBehavior,
  ]);

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
            const shouldStartEventsBridge =
              !previousRunning.current || !eventStreamActive;
            if (shouldStartEventsBridge) {
              try {
                await coreEventsStart();
              } catch (error) {
                if (!cancelled) {
                  setEventStreamActive(false);
                  setError(
                    error instanceof Error
                      ? error.message
                      : "Core 事件流重连失败",
                  );
                }
              }
            }

            if (!previousRunning.current) {
              addToast({
                title: "Core 已重连",
                description: "管理连接恢复，可继续操作。",
                variant: "default",
              });
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
  }, [
    addToast,
    eventStreamActive,
    setError,
    setEventStreamActive,
    setHeartbeatAt,
    setPhase,
    setStatus,
  ]);

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
        pushEvent(payload.payload);

        if (
          payload.payload.event === "refresh:complete" ||
          payload.payload.event === "profile:refreshed"
        ) {
          setLastRefreshAt(payload.payload.timestamp ?? new Date().toISOString());
        }

        if (
          payload.payload.event === "refresh:failed" ||
          payload.payload.event === "refresh:error" ||
          payload.payload.event === "source:degraded"
        ) {
          void notifyDesktopForCoreEvent(payload.payload);
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
  }, [addToast, pushEvent, setError, setEventStreamActive, setLastRefreshAt]);

  useEffect(() => {
    const activityEvents: Array<keyof WindowEventMap> = [
      "mousedown",
      "mousemove",
      "keydown",
      "touchstart",
      "wheel",
      "scroll",
    ];
    const markActivity = () => {
      lastActivityAt.current = Date.now();
      idleCloseTriggered.current = false;
    };

    markActivity();
    for (const eventName of activityEvents) {
      window.addEventListener(eventName, markActivity, { passive: true });
    }

    return () => {
      for (const eventName of activityEvents) {
        window.removeEventListener(eventName, markActivity);
      }
    };
  }, []);

  useEffect(() => {
    idleCloseTriggered.current = false;
    if (idleAutoCloseMinutes <= 0) {
      return;
    }

    const thresholdMs = idleAutoCloseMinutes * 60_000;
    const timer = window.setInterval(() => {
      if (document.visibilityState === "hidden" || idleCloseTriggered.current) {
        return;
      }
      const idleMs = Date.now() - lastActivityAt.current;
      if (idleMs < thresholdMs) {
        return;
      }

      idleCloseTriggered.current = true;
      void desktopAutoCloseGui();
    }, IDLE_CHECK_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [idleAutoCloseMinutes]);

  return children;
}

function parseIdleAutoCloseMinutes(rawValue: string | undefined): number {
  if (!rawValue) {
    return DEFAULT_IDLE_AUTO_CLOSE_MINUTES;
  }

  const parsed = Number.parseInt(rawValue, 10);
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 10_080) {
    return DEFAULT_IDLE_AUTO_CLOSE_MINUTES;
  }
  return parsed;
}

function parseWindowCloseBehavior(
  rawBehavior: string | undefined,
  trayMinimizeLegacyFlag: string | undefined,
): WindowCloseBehavior {
  if (
    rawBehavior === "tray_minimize" ||
    rawBehavior === "close_gui" ||
    rawBehavior === "close_gui_and_stop_core"
  ) {
    return rawBehavior;
  }
  if (parseBooleanSetting(trayMinimizeLegacyFlag)) {
    return "tray_minimize";
  }
  return "close_gui";
}

function parseBooleanSetting(rawValue: string | undefined): boolean {
  if (!rawValue) {
    return false;
  }
  return rawValue.trim().toLowerCase() === "true";
}
