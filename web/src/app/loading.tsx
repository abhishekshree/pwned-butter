export default function Loading() {
  return (
    <div className="mx-auto w-full max-w-6xl space-y-6 px-4 py-6 sm:px-6">
      {/* KPI Skeletons */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 sm:gap-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={i}
            className="h-24 animate-pulse rounded-xl border border-border/60 bg-muted/40 p-4"
          />
        ))}
      </div>

      {/* Filter Bar Skeleton */}
      <div className="h-28 animate-pulse rounded-xl border border-border/60 bg-muted/40 p-4" />

      {/* Charts Grid Skeleton */}
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <div
            key={i}
            className="h-64 animate-pulse rounded-xl border border-border/60 bg-muted/40 p-4"
          />
        ))}
      </div>

      {/* Action Cards Skeleton (2-Column Grid) */}
      <div className="grid grid-cols-1 gap-3.5 md:grid-cols-2">
        {Array.from({ length: 6 }).map((_, i) => (
          <div
            key={i}
            className="h-36 animate-pulse rounded-lg border border-border/60 bg-muted/40 p-3.5"
          />
        ))}
      </div>
    </div>
  );
}


