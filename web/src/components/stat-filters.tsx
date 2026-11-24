import Link from "next/link";
import { ArrowRight } from "lucide-react";

import { buildHref } from "@/lib/href";
import { formatNumber } from "@/lib/format";
import type { Filters } from "@/lib/types";
import { cn } from "@/lib/utils";

const TILES = [
  { status: undefined, label: "All actions", key: "all", tone: "text-foreground" },
  { status: "suspended", label: "Suspended", key: "suspended", tone: "text-status-suspended" },
  { status: "active", label: "Notices", key: "active", tone: "text-status-notice" },
  { status: "reopened", label: "Reopened", key: "reopened", tone: "text-status-reopened" },
] as const;

export function StatFilters({
  total,
  byStatus,
  filters,
}: {
  total: number;
  byStatus: { key: string; n: number }[];
  filters: Filters;
}) {
  const counts = new Map(byStatus.map((d) => [d.key, d.n]));

  return (
    <div>
      <div
        role="group"
        aria-label="Filter records by status"
        className="grid grid-cols-2 gap-2 sm:grid-cols-4"
      >
        {TILES.map((tile) => {
          const active =
            tile.status === undefined ? !filters.status : filters.status === tile.status;
          const n = tile.status === undefined ? total : (counts.get(tile.status) ?? 0);

          return (
            <Link
              key={tile.key}
              href={buildHref({ ...filters, status: tile.status })}
              aria-current={active ? "page" : undefined}
              className={cn(
                "group flex min-w-0 flex-col justify-between rounded-lg border bg-card p-3 transition-[background-color,border-color,transform] duration-150 active:translate-y-px focus-visible:ring-2 focus-visible:ring-ring",
                active
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-primary/40 hover:bg-muted/40",
              )}
            >
              <div className="flex items-start justify-between gap-1">
                <span
                  className={cn(
                    "font-heading text-2xl font-semibold leading-none tabular-nums sm:text-3xl",
                    active ? "text-primary" : tile.tone,
                  )}
                >
                  {formatNumber(n)}
                </span>
                <ArrowRight
                  className={cn(
                    "size-3.5 shrink-0 transition-colors",
                    active ? "text-primary" : "text-muted-foreground/50 group-hover:text-foreground",
                  )}
                  aria-hidden="true"
                />
              </div>
              <span
                className={cn(
                  "mt-3 truncate text-xs",
                  active ? "font-medium text-primary" : "text-muted-foreground",
                )}
              >
                {tile.label}
              </span>
            </Link>
          );
        })}
      </div>
      <p className="mt-2 text-[11px] text-muted-foreground">
        Figures cover all Maharashtra. Select one to filter.
      </p>
    </div>
  );
}
