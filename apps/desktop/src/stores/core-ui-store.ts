import { create } from "zustand";
import type {
  CoreConnectionPhase,
  CoreEventPayload,
  CoreStatus,
} from "../types/core";

type CoreUiState = {
  phase: CoreConnectionPhase;
  status: CoreStatus | null;
  error: string | null;
  heartbeatAt: string | null;
  eventStreamActive: boolean;
  lastEvent: CoreEventPayload | null;
  theme: "dark" | "light";
  toasts: ToastMessage[];
  setPhase: (phase: CoreConnectionPhase) => void;
  setStatus: (status: CoreStatus | null) => void;
  setError: (error: string | null) => void;
  setHeartbeatAt: (heartbeatAt: string | null) => void;
  setEventStreamActive: (active: boolean) => void;
  setLastEvent: (event: CoreEventPayload | null) => void;
  setTheme: (theme: "dark" | "light") => void;
  addToast: (toast: Omit<ToastMessage, "id">) => string;
  removeToast: (id: string) => void;
};

export type ToastMessage = {
  id: string;
  title: string;
  description: string;
  variant: "default" | "warning" | "error";
};

export const useCoreUiStore = create<CoreUiState>((set) => ({
  phase: "idle",
  status: null,
  error: null,
  heartbeatAt: null,
  eventStreamActive: false,
  lastEvent: null,
  theme: "dark",
  toasts: [],
  setPhase: (phase) => set({ phase }),
  setStatus: (status) => set({ status }),
  setError: (error) => set({ error }),
  setHeartbeatAt: (heartbeatAt) => set({ heartbeatAt }),
  setEventStreamActive: (eventStreamActive) => set({ eventStreamActive }),
  setLastEvent: (lastEvent) => set({ lastEvent }),
  setTheme: (theme) => set({ theme }),
  addToast: (toast) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    set((state) => ({ toasts: [...state.toasts, { ...toast, id }] }));
    return id;
  },
  removeToast: (id) =>
    set((state) => ({
      toasts: state.toasts.filter((toast) => toast.id !== id),
    })),
}));
