import Link from "next/link";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  GitFork,
  Shield,
} from "lucide-react";

import { ActionLedger } from "@/components/action-ledger";
import { Hotspots } from "@/components/hotspots";
import { SearchInput } from "@/components/search-input";
import { StatFilters } from "@/components/stat-filters";
import { ThemeToggle } from "@/components/theme-toggle";
import { buttonVariants } from "@/components/ui/button";
import { formatDateTime, formatNumber } from "@/lib/format";
import { buildHref } from "@/lib/href";
import { getStats, listActions } from "@/lib/queries";
import type { Filters } from "@/lib/types";
import { cn } from "@/lib/utils";

export const dynamic = "force-dynamic";

const PAGE_SIZE = 20;
const MAX_PAGE = 10_000;

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

  // "statewide" is the explicit all-places value; buildHref keeps it in the URL.
  const cityParam = p("city");
  const filters: Filters = {
    brand: p("brand"),
    city: cityParam ?? "Mumbai",
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
      listActions(
        {
          ...filters,
          city: filters.city === "statewide" ? undefined : filters.city,
        },
        { limit: PAGE_SIZE, offset: (page - 1) * PAGE_SIZE },
      ),
    ]);
  } catch (error) {
    console.error("Dashboard query failed", error);
    return (
      <main
        id="main"
        className="mx-auto flex w-full max-w-xl flex-1 items-center px-4 py-16"
      >
        <div className="rounded-lg border border-border bg-card p-8 text-center">
          <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-full bg-muted text-destructive">
            <AlertTriangle className="size-5" aria-hidden="true" />
          </div>
          <h1 className="font-heading text-lg font-semibold text-foreground">
            Data service unavailable
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Could not reach the database. Check the connection and try again.
          </p>
        </div>
      </main>
    );
  }

  const total = data.total;
  const totalPages = Math.ceil(total / PAGE_SIZE) || 1;
  const hasMore = page * PAGE_SIZE < total;

  return (
    <>
      <header className="sticky top-0 z-30 border-b border-border bg-background/90 backdrop-blur-md">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-3 sm:px-6">
          <Link
            href="/"
            className="flex items-center gap-2.5 rounded-sm focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="flex size-8 items-center justify-center rounded-md bg-primary font-heading text-xs font-bold text-primary-foreground">
              FDA
            </span>
            <span className="text-sm font-semibold tracking-tight text-foreground">
              Maharashtra
            </span>
          </Link>
          <div className="flex shrink-0 items-center gap-1">
            <ThemeToggle />
            <a
              href="https://github.com/abhishekshree/pwned-butter"
              target="_blank"
              rel="noopener noreferrer"
              aria-label="View source on GitHub"
              className={cn(
                buttonVariants({ variant: "ghost", size: "icon" }),
                "rounded-md",
              )}
            >
              <GitFork className="size-4" aria-hidden="true" />
            </a>
          </div>
        </div>
      </header>

      <main
        id="main"
        className="mx-auto w-full max-w-4xl flex-1 px-4 pb-12 sm:px-6"
      >
        <div className="space-y-6 pt-6 sm:space-y-7 sm:pt-8">
          <section className="border-b border-border pb-5">
            <h1 className="font-heading text-3xl font-semibold tracking-tight text-foreground text-balance sm:text-[2.5rem] sm:leading-[1.1]">
              The food safety record
            </h1>
            <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground text-pretty">
              FDA enforcement actions across Maharashtra, sourced from public
              reporting.
            </p>
            {stats.lastRun?.finished_at ? (
              <p className="mt-3 text-[11px] tabular-nums text-muted-foreground">
                Updated {formatDateTime(stats.lastRun.finished_at)}
                {stats.dateRange
                  ? `, ${stats.dateRange.days} days of records`
                  : null}
              </p>
            ) : null}
          </section>

          <StatFilters
            total={stats.totalActions}
            byStatus={stats.byStatus}
            filters={filters}
          />

          <SearchInput initial={filters.q ?? ""} filters={filters} />

          <Hotspots byCity={stats.byCity} filters={filters} />

          <section className="space-y-3">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <h2 className="font-heading text-xl font-semibold tracking-tight text-foreground">
                Enforcement ledger
              </h2>
              <div className="flex items-center gap-3">
                <p className="text-xs tabular-nums text-muted-foreground">
                  {formatNumber(total)} {total === 1 ? "record" : "records"}
                  {hasFilters ? " found" : ""}
                </p>
                {hasFilters ? (
                  <Link
                    href="/"
                    className="text-xs text-muted-foreground hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Clear
                  </Link>
                ) : null}
              </div>
            </div>

            <div id="list" className="scroll-mt-24">
              {data.actions.length ? (
                <ActionLedger actions={data.actions} />
              ) : (
                <div className="rounded-lg border border-dashed border-border bg-card/60 px-6 py-12 text-center">
                  <Shield
                    className="mx-auto mb-3 size-6 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <p className="font-heading text-lg font-semibold text-foreground">
                    No actions match these filters
                  </p>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Try a different search or clear the filters.
                  </p>
                  {hasFilters ? (
                    <Link
                      href="/"
                      className={cn(
                        buttonVariants({ variant: "outline", size: "sm" }),
                        "mt-5",
                      )}
                    >
                      Clear filters
                    </Link>
                  ) : null}
                </div>
              )}
            </div>

            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4">
              <p className="text-xs tabular-nums text-muted-foreground">
                {data.actions.length
                  ? `Showing ${(page - 1) * PAGE_SIZE + 1}–${Math.min(page * PAGE_SIZE, total)} of ${formatNumber(total)}`
                  : "No actions to show"}
              </p>
              <nav aria-label="Pagination" className="flex items-center gap-2">
                {page > 1 ? (
                  <Link
                    href={buildHref(filters, { page: page - 1 }) + "#list"}
                    className={cn(
                      buttonVariants({ variant: "outline", size: "sm" }),
                      "gap-1",
                    )}
                  >
                    <ChevronLeft className="size-3.5" aria-hidden="true" />
                    Previous
                  </Link>
                ) : (
                  <span
                    aria-disabled="true"
                    className={cn(
                      buttonVariants({ variant: "outline", size: "sm" }),
                      "gap-1 opacity-40",
                    )}
                  >
                    <ChevronLeft className="size-3.5" aria-hidden="true" />
                    Previous
                  </span>
                )}

                <span className="px-1 text-xs tabular-nums text-muted-foreground">
                  {page} / {totalPages}
                </span>

                {hasMore ? (
                  <Link
                    href={buildHref(filters, { page: page + 1 }) + "#list"}
                    className={cn(
                      buttonVariants({ variant: "outline", size: "sm" }),
                      "gap-1",
                    )}
                  >
                    Next
                    <ChevronRight className="size-3.5" aria-hidden="true" />
                  </Link>
                ) : (
                  <span
                    aria-disabled="true"
                    className={cn(
                      buttonVariants({ variant: "outline", size: "sm" }),
                      "gap-1 opacity-40",
                    )}
                  >
                    Next
                    <ChevronRight className="size-3.5" aria-hidden="true" />
                  </span>
                )}
              </nav>
            </div>
          </section>

          <footer className="border-t border-border pt-6 text-xs leading-relaxed text-muted-foreground">
            <p className="max-w-2xl">
              Unofficial compilation from public news reporting, not an FDA
              register. Every record links to its source.
            </p>
            <p className="mt-2">
              Built by{" "}
              <a
                href="https://abhishekshree.github.io"
                target="_blank"
                rel="noopener noreferrer"
                className="font-medium text-foreground underline underline-offset-2 hover:no-underline"
              >
                Abhishek Shree
              </a>{" "}
              · source on{" "}
              <a
                href="https://github.com/abhishekshree/pwned-butter"
                target="_blank"
                rel="noopener noreferrer"
                className="font-medium text-foreground underline underline-offset-2 hover:no-underline"
              >
                GitHub
              </a>
            </p>
          </footer>
        </div>
      </main>
    </>
  );
}
