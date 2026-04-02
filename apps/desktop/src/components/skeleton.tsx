type SkeletonProps = {
  className?: string;
};

export function Skeleton({ className = "" }: SkeletonProps) {
  return (
    <div
      className={`animate-pulse rounded-md bg-[color:var(--panel-muted)] ${className}`}
      aria-hidden
    />
  );
}
