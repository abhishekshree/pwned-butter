import Link from "next/link";
import {
  Activity,
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock,
  GitFork,
  Radio,
  RotateCcw,
  Shield,
  ShieldAlert,
} from "lucide-react";

import { ActionCard } from "@/components/action-card";
import { BarChartCard, DonutChart } from "@/components/charts";
import { Controls } from "@/components/filters";
import { ThemeToggle } from "@/components/theme-toggle";
import { Badge } from "@/components/ui/badge";
import { buttonVariants } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { formatDateTime, formatNumber } from "@/lib/format";
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
const MAX_PAGE = 10_000;

const STATUS_CHIPS: { v: string | null; label: string; tone: string }[] = [
  { v: null, label: "All Actions", tone: "border-primary/40 bg-primary/10 text-foreground" },
  { v: "suspended", label: "Suspended", tone: "border-red-500/40 bg-red-500/10 text-red-600 dark:text-red-400" },
  { v: "reopened", label: "Reopened", tone: "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400" },
  { v: "active", label: "Active / Notices", tone: "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400" },
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
  let brands: string[];
  let cities: string[];
  let data: { total: number; actions: import("@/lib/types").ActionRow[] };
  try {
    [stats, brands, cities, data] = await Promise.all([
      getStats(),
      listBrands(),
      listCities(),
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
  const activeCount = byStatus.get("active") ?? 0;

  const kpis = [
    {
      label: "TOTAL ACTIONS",
      value: formatNumber(totalActions),
      sub: "Logged enforcement actions",
      icon: Activity,
      border: "border-primary/20 hover:border-primary/40",
      indicator: "bg-blue-500",
      accent: "text-blue-600 dark:text-blue-400",
    },
    {
      label: "SUSPENDED",
      value: formatNumber(suspendedCount),
      sub: `${totalActions > 0 ? Math.round((suspendedCount / totalActions) * 100) : 0}% of all recorded actions`,
      icon: AlertTriangle,
      border: "border-red-500/20 hover:border-red-500/40",
      indicator: "bg-red-500",
      accent: "text-red-600 dark:text-red-400",
    },
    {
      label: "REOPENED",
      value: formatNumber(reopenedCount),
      sub: "Cleared post-compliance verification",
      icon: CheckCircle2,
      border: "border-emerald-500/20 hover:border-emerald-500/40",
      indicator: "bg-emerald-500",
      accent: "text-emerald-600 dark:text-emerald-400",
    },
    {
      label: "NOTICES & RAIDS",
      value: formatNumber(activeCount),
      sub: "Active inspection hearings & seizures",
      icon: ShieldAlert,
      border: "border-amber-500/20 hover:border-amber-500/40",
      indicator: "bg-amber-500",
      accent: "text-amber-600 dark:text-amber-400",
    },
  ];

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
      <div className="mx-auto max-w-6xl space-y-6 px-4 py-6 sm:px-6">
        {/* KPI Metrics Grid */}
        <section className="grid grid-cols-2 gap-3 sm:grid-cols-4 sm:gap-4">
          {kpis.map((k) => {
            const Icon = k.icon;
            return (
              <div
                key={k.label}
                className={cn(
                  "group relative flex flex-col justify-between rounded-xl border bg-card p-3.5 sm:p-4 shadow-xs transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md dark:bg-card/90",
                  k.border,
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
                    {k.label}
                  </span>
                  <span className={cn("size-2 rounded-full", k.indicator)} />
                </div>
                <div className="mt-2">
                  <div className="font-mono text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
                    {k.value}
                  </div>
                  <div className="mt-1 line-clamp-1 text-[11px] text-muted-foreground">
                    {k.sub}
                  </div>
                </div>
              </div>
            );
          })}
        </section>

        {/* Filters and Search Strip */}
        <section className="rounded-xl border border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
          <Controls key={href} initial={filters} brands={brands} cities={cities} />

          {/* Status Tabs & Active Filters */}
          <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-border/50 pt-3">
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
        </section>

        {/* Visual Analytics / Charts Grid */}
        <section className="grid grid-cols-1 gap-4 md:grid-cols-3">
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <DonutChart title="Actions by Status" data={stats.byStatus} tag="// STATUS RATIO" />
          </Card>
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <BarChartCard title="Enforcement Actions" data={stats.byActionType} tag="// ACTION TYPES" />
          </Card>
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <BarChartCard title="Top Regional Centers" data={stats.byCity} tag="// TOP CITIES" />
          </Card>
        </section>

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
            {stats.lastRun?.finished_at ? (
              <span className="hidden sm:inline-flex items-center gap-1">
                <Clock className="size-3 text-muted-foreground/70" />
                Updated {formatDateTime(stats.lastRun.finished_at)}
              </span>
            ) : null}
            <span>
              Page {page} of {totalPages}
            </span>
          </div>
        </div>

        {/* Action Cards Feed (2-Column Grid) */}
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
        <div className="flex items-center justify-between gap-3 border-t border-border/60 pt-4 pb-6">
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

        {/* Footer */}
        <footer className="border-t border-border/80 pt-6 pb-12 text-xs">
          <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <p className="max-w-xl leading-relaxed text-muted-foreground">
              An independent, unofficial compilation of food-safety enforcement actions across
              Maharashtra, aggregated from public news reporting. Not affiliated with or endorsed
              by any government body. Use as a reference only — data is news-derived and may
              contain errors, and is not the definitive truth.
            </p>
            <div className="flex flex-wrap gap-2">
              <span className="rounded-md border border-border/80 bg-card px-2 py-1 font-mono text-[11px] text-muted-foreground">
                ⚡ Rust Engine
              </span>
              <span className="rounded-md border border-border/80 bg-card px-2 py-1 font-mono text-[11px] text-muted-foreground">
                ☁️ Next.js 16
              </span>
              <span className="rounded-md border border-border/80 bg-card px-2 py-1 font-mono text-[11px] text-muted-foreground">
                🐘 Neon Postgres
              </span>
            </div>
          </div>
          <div className="mt-6 border-t border-border/40 pt-4 text-muted-foreground">
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
          </div>
        </footer>
      </div>
    </main>
  );
}

