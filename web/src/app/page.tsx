import Link from "next/link";
import { GitFork } from "lucide-react";

import { ActionCard } from "@/components/action-card";
import { BarChartCard, DonutChart } from "@/components/charts";
import { Controls } from "@/components/filters";
import { ThemeToggle } from "@/components/theme-toggle";
import { Badge } from "@/components/ui/badge";
import { buttonVariants } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { formatDateTime } from "@/lib/format";
import { buildHref } from "@/lib/href";
import {
  getStats,
  listActions,
  listBrands,
  listCities,
} from "@/lib/queries";
import type { Filters } from "@/lib/types";
import { cn } from "@/lib/utils";

export const dynamic = "force-dynamic";

const PAGE_SIZE = 20;

const STATUS_CHIPS: { v: string | null; label: string }[] = [
  { v: null, label: "All" },
  { v: "suspended", label: "Suspended" },
  { v: "reopened", label: "Reopened" },
  { v: "active", label: "Active / notices" },
];

function single(v: string | string[] | undefined): string | undefined {
  return typeof v === "string" ? v : undefined;
}

export default async function Home({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const sp = await searchParams;
  const p = (k: string) => single(sp[k]);
  const page = Math.max(1, Math.min(10, Number(p("page")) || 1));

  const filters: Filters = {
    brand: p("brand"),
    city: p("city"),
    status: p("status"),
    action_type: p("action_type"),
    q: p("q"),
  };
  const href = buildHref(filters);
  const hasFilters = href !== "/";

  let stats;
  let brands: string[];
  let cities: string[];
  let data: { total: number; actions: import("@/lib/types").ActionRow[] };
  try {
    [stats, brands, cities, data] = await Promise.all([
      getStats(),
      listBrands(),
      listCities(),
      listActions(filters, { limit: page * PAGE_SIZE, offset: 0 }),
    ]);
  } catch (err) {
    return (
      <main className="mx-auto flex w-full max-w-xl flex-1 items-center px-4">
        <div className="rounded-2xl border bg-card px-8 py-10 text-center shadow-sm">
          <div className="text-4xl">🧺</div>
          <h1 className="mt-3 text-lg font-semibold text-foreground">
            Couldn&apos;t load the tracker
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The database isn&apos;t reachable right now ({(err as Error).message}).
          </p>
        </div>
      </main>
    );
  }

  const byStatus = new Map(stats.byStatus.map((d) => [d.key, d.n]));
  const tiles = [
    { label: "Total actions", value: stats.totalActions, dot: "bg-blue-500" },
    { label: "Suspended", value: byStatus.get("suspended") ?? 0, dot: "bg-red-500" },
    { label: "Reopened", value: byStatus.get("reopened") ?? 0, dot: "bg-emerald-500" },
    { label: "Active / notices", value: byStatus.get("active") ?? 0, dot: "bg-amber-500" },
  ];

  const total = data.total;
  const shown = data.actions.length;
  const hasMore = shown < total;

  return (
    <main>
      <header className="sticky top-0 z-30 border-b bg-background/90 backdrop-blur">
        <div className="mx-auto flex max-w-3xl items-center justify-between gap-2 px-4 py-3">
          <div className="flex min-w-0 items-center gap-2.5">
            <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-blue-500 to-blue-700 text-[11px] font-extrabold tracking-wide text-white shadow-sm">
              FDA
            </span>
            <h1 className="truncate text-base font-semibold tracking-tight">
              Maharashtra FDA Enforcement Tracker
            </h1>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <ThemeToggle />
            <a
              href="https://github.com/abhishekshree/fda-mumbai-tracker"
              target="_blank"
              rel="noopener"
              aria-label="Source on GitHub"
              className={buttonVariants({ variant: "ghost", size: "icon" })}
            >
              <GitFork className="size-4" />
            </a>
          </div>
        </div>
      </header>

      <div className="mx-auto max-w-3xl px-4">
        <section className="py-5">
          <p className="text-sm text-muted-foreground">
            Daily log of food-safety actions across Maharashtra — licence
            suspensions, raids, seals and more, compiled from Indian news.
          </p>

          <div className="mt-3 grid grid-cols-2 gap-2.5 sm:grid-cols-4">
            {tiles.map((t) => (
              <div key={t.label} className="rounded-xl border bg-card p-3 shadow-sm">
                <div className="text-xl font-bold tracking-tight">{t.value}</div>
                <div className="mt-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                  <span className={cn("size-1.5 rounded-full", t.dot)} />
                  {t.label}
                </div>
              </div>
            ))}
          </div>
        </section>

        <section className="pb-5">
          <Controls key={href} initial={filters} brands={brands} cities={cities} />

          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {STATUS_CHIPS.map((chip) => {
              const active =
                chip.v === null ? !filters.status : filters.status === chip.v;
              const count =
                chip.v === null ? stats.totalActions : (byStatus.get(chip.v) ?? 0);
              return (
                <Link
                  key={chip.v ?? "all"}
                  href={buildHref({ ...filters, status: chip.v ?? undefined })}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "rounded-full border px-3 py-1.5 text-xs font-semibold transition",
                    active
                      ? "border-primary bg-primary text-primary-foreground"
                      : "border-border bg-card text-muted-foreground hover:text-foreground",
                  )}
                >
                  {chip.label}
                  <span
                    className={cn(
                      "ml-1.5 rounded-full px-1.5 text-[10px] font-bold",
                      active
                        ? "bg-primary-foreground/20 text-primary-foreground"
                        : "bg-muted text-muted-foreground",
                    )}
                  >
                    {count}
                  </span>
                </Link>
              );
            })}
            {hasFilters ? (
              <Link
                href="/"
                className="ml-1 text-xs font-semibold text-blue-500 underline-offset-2 hover:underline dark:text-blue-400"
              >
                Clear filters
              </Link>
            ) : null}
          </div>
        </section>

        <section className="grid gap-3 pb-5 sm:grid-cols-2 lg:grid-cols-3">
          <Card className="p-4">
            <DonutChart title="By status" data={stats.byStatus} />
          </Card>
          <Card className="p-4 sm:col-span-2 lg:col-span-1">
            <BarChartCard title="By action" data={stats.byActionType} />
          </Card>
          <Card className="p-4 sm:col-span-2 lg:col-span-1">
            <BarChartCard title="By city" data={stats.byCity} />
          </Card>
        </section>

        <div className="flex items-baseline justify-between gap-2 pb-3 text-xs text-muted-foreground">
          <div>
            {total} record{total === 1 ? "" : "s"}
            {hasFilters ? ` · filtered` : ""}
          </div>
          {stats.lastRun?.finished_at ? (
            <div className="truncate">Scraped {formatDateTime(stats.lastRun.finished_at)}</div>
          ) : null}
        </div>

        <section id="list" className="scroll-mt-24 space-y-3 pb-3">
          {data.actions.length ? (
            data.actions.map((a) => <ActionCard key={a.id} a={a} />)
          ) : (
            <div className="rounded-2xl border border-dashed p-12 text-center">
              <div className="text-3xl">🧺</div>
              <p className="mt-2 text-sm font-medium">No matching actions</p>
              <p className="text-xs text-muted-foreground">
                Try a different search or clear the filters.
              </p>
            </div>
          )}
        </section>

        <div className="flex items-center justify-center gap-3 pb-6">
          {page > 1 ? (
            <Link
              href={href + "#list"}
              className="text-xs font-semibold text-muted-foreground hover:text-foreground"
            >
              ← Back to latest
            </Link>
          ) : null}
          {hasMore ? (
            <Link
              href={buildHref(filters, { page: page + 1 }) + "#list"}
              className={buttonVariants({ variant: "outline" })}
            >
              Load more · {total - shown} more
            </Link>
          ) : null}
        </div>

        <footer className="border-t pb-10 pt-4">
          <p className="max-w-prose text-xs leading-relaxed text-muted-foreground">
            Compiled automatically from public news reports — not an official
            register, and the data is news-derived and may contain errors. A
            suspension is not a cancellation: outlets often reopen after
            compliance. Every report links to its source article.
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <Badge variant="outline">Rust · Vercel · Neon</Badge>
            <Badge variant="outline">Compiled daily 06:00 UTC</Badge>
          </div>
        </footer>
      </div>
    </main>
  );
}