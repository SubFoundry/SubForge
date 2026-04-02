import * as Toast from "@radix-ui/react-toast";
import { useEffect } from "react";
import { useCoreUiStore } from "../stores/core-ui-store";

const VARIANT_STYLE = {
  default: "border-cyan-300/30 bg-cyan-950/80 text-cyan-50",
  warning: "border-amber-300/30 bg-amber-950/70 text-amber-50",
  error: "border-rose-300/35 bg-rose-950/75 text-rose-50",
} as const;

export function ToastHost() {
  const toasts = useCoreUiStore((state) => state.toasts);
  const removeToast = useCoreUiStore((state) => state.removeToast);

  useEffect(() => {
    if (toasts.length === 0) {
      return;
    }
    const timers = toasts.map((toast) =>
      window.setTimeout(() => removeToast(toast.id), 3500),
    );
    return () => {
      timers.forEach((timer) => window.clearTimeout(timer));
    };
  }, [removeToast, toasts]);

  return (
    <Toast.Provider swipeDirection="right">
      {toasts.map((toast) => (
        <Toast.Root
          key={toast.id}
          open
          onOpenChange={(open) => {
            if (!open) {
              removeToast(toast.id);
            }
          }}
          className={`mb-2 w-[320px] rounded-xl border px-4 py-3 shadow-2xl backdrop-blur ${VARIANT_STYLE[toast.variant]}`}
        >
          <Toast.Title className="text-sm font-semibold">{toast.title}</Toast.Title>
          <Toast.Description className="mt-1 text-xs opacity-90">
            {toast.description}
          </Toast.Description>
        </Toast.Root>
      ))}
      <Toast.Viewport className="fixed bottom-4 right-4 z-50" />
    </Toast.Provider>
  );
}
