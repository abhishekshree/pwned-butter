import Link from "next/link";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Clock,
  GitFork,
  RotateCcw,
  Shield,
} from "lucide-react";

import { ActionCard } from "@/components/action-card";
import { Analytics } from "@/components/analytics";
import { AreaMap } from "@/components/area-map";
import { SearchInput } from "@/components/search-input";
import { ThemeToggle } from "@/components/theme-toggle";
import { buttonVariants } from "@/components/ui/button";
import { formatDate, formatDateTime, formatNumber, STATUS_CONFIG } from "@/lib/format";
import { buildHref } from "@/lib/href";
import {
  getStats,
  listActions,
} from "@/lib/queries";
import type { Filters } from "@/lib/types";
import { cn } from "@/lib/utils";

export const dynamic = "force-dynamic";

const PAGE_SIZE = 20;
const MAX_PAGE = 10_000;

const STATUS_CHIPS: { v: string | null; label: string }[] = [
  { v: null, label: "All Actions" },
  ...Object.entries(STATUS_CONFIG).map(([v, cfg]) => ({ v, label: cfg.chipLabel })),
];

function single(v: string | string[] | undefined): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function pageNumber(v: string | undefined): number {
  if (!v || !/^\d+$/.test(v)) return 1;
  return Math.min(Math.max(Number(v), 1), MAX_PAGE);
}

export default async function Home({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const sp = await searchParams;
  const p = (k: string) => single(sp[k]);
  const page = pageNumber(p("page"));

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
  let data: { total: number; actions: import("@/lib/types").ActionRow[] };
  try {
    [stats, data] = await Promise.all([
      getStats(),
      listActions(filters, { limit: PAGE_SIZE, offset: (page - 1) * PAGE_SIZE }),
    ]);
  } catch (error) {
    console.error("Dashboard query failed", error);
    return (
      <main className="mx-auto flex w-full max-w-xl flex-1 items-center px-4 py-16">
        <div className="rounded-xl border border-destructive/30 bg-card p-8 text-center shadow-lg">
          <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-full bg-destructive/10 text-destructive">
            <AlertTriangle className="size-5" />
          </div>
          <h1 className="text-base font-semibold text-foreground">
            Data Service Unavailable
          </h1>
          <p className="mt-1 font-mono text-xs text-muted-foreground">
            Could not reach the database tracker. Please verify connection credentials.
          </p>
        </div>
      </main>
    );
  }

  const byStatus = new Map(stats.byStatus.map((d) => [d.key, d.n]));
  const total = data.total;
  const totalPages = Math.ceil(total / PAGE_SIZE) || 1;
  const hasMore = page * PAGE_SIZE < total;

  const totalActions = stats.totalActions;
  const suspendedCount = byStatus.get("suspended") ?? 0;
  const reopenedCount = byStatus.get("reopened") ?? 0;

  return (
    <main className="min-h-screen">
      {/* Sticky Top Navigation */}
      <header className="sticky top-0 z-30 border-b border-border/80 bg-background/80 backdrop-blur-md">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-3 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-primary/30 bg-primary/10 font-mono text-xs font-bold text-primary shadow-xs">
              FDA
            </span>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="truncate text-sm font-bold tracking-tight text-foreground sm:text-base">
                  Is the Mumbai Butter Real?
                </h1>
                <span className="inline-flex items-center gap-1 rounded border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-emerald-600 dark:text-emerald-400">
                  <span className="size-1.5 rounded-full bg-emerald-500 animate-pulse" />
                  LIVE
                </span>
              </div>
              <p className="truncate font-mono text-[11px] text-muted-foreground">
                live FDA raids, suspensions &amp; seizures across Maharashtra
                {stats.dateRange
                  ? ` · ${stats.dateRange.days}d of data (${formatDate(stats.dateRange.from)} – ${formatDate(stats.dateRange.to)})`
                  : null}
              </p>
            </div>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            <span className="hidden rounded-lg border border-border/80 bg-muted/40 px-2.5 py-1 font-mono text-xs text-muted-foreground sm:inline-flex">
              <span className="font-semibold text-foreground mr-1">{formatNumber(stats.totalActions)}</span> entries
            </span>
            <ThemeToggle />
            <a
              href="https://github.com/abhishekshree/pwned-butter"
              target="_blank"
              rel="noopener noreferrer"
              aria-label="View source repository on GitHub"
              className={cn(
                buttonVariants({ variant: "outline", size: "icon" }),
                "rounded-lg border-border hover:bg-muted",
              )}
            >
              <GitFork className="size-4" />
            </a>
          </div>
        </div>
      </header>

      {/* Main Container */}
      <div className="mx-auto max-w-6xl space-y-5 px-4 py-6 sm:px-6">
        {/* Stats + map (two columns) */}
        <section className="grid grid-cols-1 gap-4 md:grid-cols-3">
          <div className="flex flex-col justify-center gap-3 rounded-xl border border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <div className="flex items-baseline justify-between gap-2">
              <span className="text-sm text-muted-foreground">Total actions</span>
              <span className="font-mono text-2xl font-bold text-foreground">{formatNumber(totalActions)}</span>
            </div>
            <div className="flex items-baseline justify-between gap-2 border-t border-border/50 pt-3">
              <span className="text-sm text-muted-foreground">Suspended</span>
              <span className="font-mono text-2xl font-bold text-red-600 dark:text-red-400">{formatNumber(suspendedCount)}</span>
            </div>
            <div className="flex items-baseline justify-between gap-2 border-t border-border/50 pt-3">
              <span className="text-sm text-muted-foreground">Reopened</span>
              <span className="font-mono text-2xl font-bold text-emerald-600 dark:text-emerald-400">{formatNumber(reopenedCount)}</span>
            </div>
            {stats.lastRun?.finished_at ? (
              <p className="inline-flex items-center gap-1 border-t border-border/50 pt-3 font-mono text-xs text-muted-foreground">
                <Clock className="size-3 text-muted-foreground/70" />
                Updated {formatDateTime(stats.lastRun.finished_at)} ·{" "}
                {stats.lastRun.actions_upserted} row
                {stats.lastRun.actions_upserted === 1 ? "" : "s"}
              </p>
            ) : null}
          </div>

          <div className="md:col-span-2">
            <AreaMap data={stats.byArea} filters={filters} />
          </div>
        </section>

        {/* Name search */}
        <SearchInput initial={filters.q ?? ""} filters={filters} />

        {/* Status Tabs & Active Filters */}
        <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-[10px] font-semibold uppercase text-muted-foreground mr-1">
                  Status:
                </span>
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
                        "inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs font-medium transition-colors",
                        active
                          ? "border-primary bg-primary text-primary-foreground font-semibold shadow-xs"
                          : "border-border bg-muted/40 text-muted-foreground hover:bg-muted hover:text-foreground",
                      )}
                    >
                      <span>{chip.label}</span>
                      <span
                        className={cn(
                          "rounded px-1 font-mono text-[10px] font-bold",
                          active
                            ? "bg-primary-foreground/20 text-primary-foreground"
                            : "bg-background text-muted-foreground",
                        )}
                      >
                        {formatNumber(count)}
                      </span>
                    </Link>
                  );
                })}
              </div>

              {hasFilters ? (
                <Link
                  href="/"
                  className="inline-flex items-center gap-1 font-mono text-xs font-medium text-primary hover:underline"
                >
                  <RotateCcw className="size-3" />
                  <span>Clear all filters</span>
                </Link>
              ) : null}
        </div>

        {/* Action Feed Section Header */}
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-3">
          <div className="flex items-center gap-2">
            <h2 className="text-base font-bold tracking-tight text-foreground">
              Enforcement Log
            </h2>
            <span className="rounded-md border border-border/80 bg-muted/50 px-2 py-0.5 font-mono text-xs font-semibold text-muted-foreground">
              {formatNumber(total)} {total === 1 ? "record" : "records"}
              {hasFilters ? " (filtered)" : ""}
            </span>
          </div>

          <div className="flex items-center gap-3 font-mono text-xs text-muted-foreground">
            <span>
              Page {page} of {totalPages}
            </span>
          </div>
        </div>

        {/* Action Cards Feed (single column, centered) */}
        <section id="list" className="scroll-mt-20">
          {data.actions.length ? (
            <div className="grid grid-cols-1 gap-3.5 md:grid-cols-2">
              {data.actions.map((a) => (
                <ActionCard key={a.id} a={a} />
              ))}
            </div>
          ) : (
            <div className="rounded-xl border border-dashed border-border bg-card/50 p-10 text-center">
              <div className="mx-auto mb-2 flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
                <Shield className="size-5" />
              </div>
              <p className="text-sm font-semibold text-foreground">No matching enforcement actions</p>
              <p className="mt-1 font-mono text-xs text-muted-foreground">
                Try modifying your search query or selecting &quot;All Actions&quot;.
              </p>
              {hasFilters ? (
                <div className="mt-4">
                  <Link
                    href="/"
                    className={cn(buttonVariants({ variant: "outline", size: "sm" }), "font-mono text-xs")}
                  >
                    Reset all filters
                  </Link>
                </div>
              ) : null}
            </div>
          )}
        </section>

        {/* Pagination Controls */}
        <div className="flex items-center justify-between gap-3 border-t border-border/60 pt-4 pb-2">
          <div className="font-mono text-xs text-muted-foreground">
            Showing {data.actions.length > 0 ? (page - 1) * PAGE_SIZE + 1 : 0}–
            {Math.min(page * PAGE_SIZE, total)} of {formatNumber(total)}
          </div>

          <div className="flex items-center gap-2">
            {page > 1 ? (
              <Link
                href={buildHref(filters, { page: page - 1 }) + "#list"}
                className={cn(
                  buttonVariants({ variant: "outline", size: "sm" }),
                  "inline-flex items-center gap-1 font-mono text-xs",
                )}
              >
                <ChevronLeft className="size-3.5" />
                <span>Previous</span>
              </Link>
            ) : (
              <button
                disabled
                className={cn(
                  buttonVariants({ variant: "outline", size: "sm" }),
                  "inline-flex items-center gap-1 font-mono text-xs opacity-50 cursor-not-allowed",
                )}
              >
                <ChevronLeft className="size-3.5" />
                <span>Previous</span>
              </button>
            )}

            <span className="px-2 font-mono text-xs font-semibold text-foreground">
              {page} / {totalPages}
            </span>

            {hasMore ? (
              <Link
                href={buildHref(filters, { page: page + 1 }) + "#list"}
                className={cn(
                  buttonVariants({ variant: "outline", size: "sm" }),
                  "inline-flex items-center gap-1 font-mono text-xs",
                )}
              >
                <span>Next</span>
                <ChevronRight className="size-3.5" />
              </Link>
            ) : (
              <button
                disabled
                className={cn(
                  buttonVariants({ variant: "outline", size: "sm" }),
                  "inline-flex items-center gap-1 font-mono text-xs opacity-50 cursor-not-allowed",
                )}
              >
                <span>Next</span>
                <ChevronRight className="size-3.5" />
              </button>
            )}
          </div>
        </div>

        {/* Analytics (collapsed — charts mount only when opened) */}
        <Analytics
          byStatus={stats.byStatus}
          byActionType={stats.byActionType}
          byCity={stats.byCity}
        />

        {/* Footer */}
        <footer className="border-t border-border/80 pt-6 pb-12 text-xs text-muted-foreground">
          <p className="max-w-3xl leading-relaxed">
            Unofficial compilation of Maharashtra food-safety enforcement from public news
            reporting — not an FDA register, may contain errors. Every record links to its source.
          </p>
          <p className="mt-3">
            Built by{" "}
            <a
              href="https://abhishekshree.github.io"
              target="_blank"
              rel="noopener noreferrer"
              className="font-medium text-foreground underline underline-offset-2 decoration-border hover:decoration-foreground"
            >
              Abhishek Shree
            </a>{" "}
            · source on{" "}
            <a
              href="https://github.com/abhishekshree/pwned-butter"
              target="_blank"
              rel="noopener noreferrer"
              className="font-medium text-foreground underline underline-offset-2 decoration-border hover:decoration-foreground"
            >
              GitHub
            </a>
          </p>
        </footer>
      </div>
    </main>
  );
}
