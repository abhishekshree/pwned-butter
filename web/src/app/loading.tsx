export default function Loading() {
  return (
    <div className="mx-auto w-full max-w-3xl px-4 py-6">
      <div className="h-4 w-3/4 animate-pulse rounded bg-muted" />
      <div className="mt-3 grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-muted" />
        ))}
      </div>
      <div className="mt-5 h-11 animate-pulse rounded-xl bg-muted" />
      <div className="mt-4 space-y-3">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="h-40 animate-pulse rounded-2xl bg-muted" />
        ))}
      </div>
    </div>
  );
}