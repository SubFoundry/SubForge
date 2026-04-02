type StatusIndicatorProps = {
  status: "online" | "degraded" | "offline";
  label: string;
};

const STATUS_STYLE: Record<StatusIndicatorProps["status"], string> = {
  online:
    "bg-emerald-400/20 text-emerald-100 ring-1 ring-emerald-300/30 before:bg-emerald-300",
  degraded:
    "bg-amber-300/20 text-amber-50 ring-1 ring-amber-300/30 before:bg-amber-200",
  offline:
    "bg-rose-400/20 text-rose-100 ring-1 ring-rose-300/35 before:bg-rose-300",
};

export function StatusIndicator({ status, label }: StatusIndicatorProps) {
  return (
    <span
      className={`inline-flex items-center gap-2 rounded-full px-3 py-1 text-xs font-semibold tracking-wide ${STATUS_STYLE[status]} before:h-2 before:w-2 before:rounded-full`}
    >
      {label}
    </span>
  );
}
