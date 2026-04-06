import type { ReactNode } from "react";
import { Skeleton } from "./skeleton";

type StatePanelVariant = "loading" | "error" | "empty" | "success";

const VARIANT_CLASS: Record<StatePanelVariant, string> = {
  loading: "ui-state-loading",
  error: "ui-state-error",
  empty: "ui-state-empty",
  success: "ui-state-success",
};

export function StatePanel({
  variant,
  title,
  description,
  actions,
}: {
  variant: StatePanelVariant;
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <div className={`ui-state ${VARIANT_CLASS[variant]}`}>
      <div className="ui-state-copy">
        <p className="ui-state-title">{title}</p>
        <p className="ui-state-desc">{description}</p>
      </div>
      {actions ? <div className="ui-state-actions">{actions}</div> : null}
    </div>
  );
}

export function StateSkeletonRows({ rows = 2 }: { rows?: number }) {
  return (
    <div className="space-y-2">
      {Array.from({ length: rows }).map((_, index) => (
        <Skeleton key={`skeleton-${index}`} className="h-20" />
      ))}
    </div>
  );
}
