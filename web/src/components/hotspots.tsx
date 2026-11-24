import Link from "next/link";

import { buildHref } from "@/lib/href";
import { humanize } from "@/lib/format";
import type { DimCount, Filters } from "@/lib/types";
import { cn } from "@/lib/utils";

export function Hotspots({
  byCity,
  filters,
}: {
  byCity: DimCount[];
  filters: Filters;
}) {
  const cities = byCity.slice(0, 5);

  return (
    <div>
      <p className="mb-2 text-xs text-muted-foreground">Where</p>
      <div className="flex gap-2 overflow-x-auto pb-1 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <Link
          href={buildHref({ ...filters, city: "statewide" })}
          aria-current={filters.city === "statewide" ? "page" : undefined}
          className={cn(
            "shrink-0 rounded-full border px-3 py-1.5 text-xs transition-colors focus-visible:ring-2 focus-visible:ring-ring",
            filters.city === "statewide"
              ? "border-primary bg-primary text-primary-foreground"
              : "border-border bg-card text-muted-foreground hover:border-primary/40 hover:text-foreground",
          )}
        >
          All places
        </Link>
        {cities.map((c) => {
          const active = filters.city === c.key;
          return (
            <Link
              key={c.key}
              href={buildHref({ ...filters, city: c.key })}
              aria-current={active ? "page" : undefined}
              title={`${humanize(c.key)}: ${c.n} actions`}
              className={cn(
                "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                active
                  ? "border-primary bg-primary text-primary-foreground"
                  : "border-border bg-card text-muted-foreground hover:border-primary/40 hover:text-foreground",
              )}
            >
              <span className="max-w-32 truncate">{humanize(c.key)}</span>
              <span className={cn("tabular-nums", active ? "opacity-80" : "opacity-60")}>
                {c.n}
              </span>
            </Link>
          );
        })}
      </div>
    </div>
  );
}
