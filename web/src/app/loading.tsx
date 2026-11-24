export default function Loading() {
  return (
    <div className="mx-auto w-full max-w-4xl flex-1 space-y-6 px-4 pt-6 pb-12 sm:space-y-7 sm:px-6 sm:pt-8">
      <p className="sr-only">Loading enforcement records…</p>
      <div className="h-28 animate-pulse border-b border-border" />
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={i}
            className="h-20 animate-pulse rounded-lg border border-border bg-muted/40"
          />
        ))}
      </div>
      <div className="h-11 animate-pulse rounded-lg bg-muted/50" />
      <div className="h-8 animate-pulse" />
      <div className="divide-y divide-border rounded-lg border border-border">
        {Array.from({ length: 10 }).map((_, i) => (
          <div key={i} className="h-14 animate-pulse px-4 py-3">
            <div className="h-4 w-full rounded bg-muted/50" />
          </div>
        ))}
      </div>
    </div>
  );
}
