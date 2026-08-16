"use client";

import { useEffect, useRef, useState } from "react";
import { ChevronDown, Search, X } from "lucide-react";
import { useRouter } from "next/navigation";

import { ACTION_TYPE_LABELS } from "@/lib/format";
import { activeFilter, buildHref } from "@/lib/href";
import type { Filters } from "@/lib/types";

const selectCls =
  "h-10 w-full appearance-none rounded-lg border border-border bg-card pr-8 pl-3 text-xs font-medium text-foreground outline-none transition-colors hover:border-primary/50 focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring sm:w-auto dark:bg-card/80";

function selectValue(v: string | undefined, fallback: string): string {
  return activeFilter(v) ?? fallback;
}

export function Controls({
  initial,
  brands,
  cities,
}: {
  initial: Filters;
  brands: string[];
  cities: string[];
}) {
  const router = useRouter();
  const [brand, setBrand] = useState(selectValue(initial.brand, "all"));
  const [city, setCity] = useState(selectValue(initial.city, "all"));
  const [actionType, setActionType] = useState(selectValue(initial.action_type, "all"));
  const [q, setQ] = useState(initial.q ?? "");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const apply = (next: Filters) => router.replace(buildHref(next));

  const onSelect = (key: keyof Filters, value: string) => {
    if (timer.current) clearTimeout(timer.current);
    const v = value === "all" ? undefined : value;
    apply({ ...initial, q: q.trim() || undefined, [key]: v });
  };

  const onSearch = (value: string) => {
    setQ(value);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      apply({ ...initial, q: value.trim() || undefined });
    }, 300);
  };

  const clearSearch = () => {
    setQ("");
    apply({ ...initial, q: undefined });
  };

  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-center">
      <div className="relative flex-1">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <input
          type="search"
          value={q}
          onChange={(e) => onSearch(e.target.value)}
          placeholder="Filter establishments, areas, brands... (e.g. Bandra, Cloud Kitchen)"
          className="h-10 w-full rounded-lg border border-border bg-card pr-9 pl-9 text-xs sm:text-sm text-foreground outline-none transition-colors placeholder:text-muted-foreground/70 hover:border-primary/50 focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring dark:bg-card/80"
        />
        {q ? (
          <button
            type="button"
            onClick={clearSearch}
            aria-label="Clear search"
            className="absolute top-1/2 right-2.5 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        ) : (
          <kbd className="pointer-events-none absolute top-1/2 right-2.5 hidden -translate-y-1/2 select-none items-center rounded border border-border bg-muted/60 px-1.5 font-mono text-[10px] font-medium text-muted-foreground sm:inline-flex">
            /
          </kbd>
        )}
      </div>

      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap sm:items-center">
        <div className="relative w-full sm:w-auto">
          <select
            aria-label="Filter by brand"
            value={brand}
            onChange={(e) => {
              setBrand(e.target.value);
              onSelect("brand", e.target.value);
            }}
            className={selectCls}
          >
            <option value="all">All Brands ({brands.length})</option>
            {brands.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        </div>

        <div className="relative w-full sm:w-auto">
          <select
            aria-label="Filter by city"
            value={city}
            onChange={(e) => {
              setCity(e.target.value);
              onSelect("city", e.target.value);
            }}
            className={selectCls}
          >
            <option value="all">All Cities ({cities.length})</option>
            {cities.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        </div>

        <div className="relative w-full sm:w-auto">
          <select
            aria-label="Filter by action"
            value={actionType}
            onChange={(e) => {
              setActionType(e.target.value);
              onSelect("action_type", e.target.value);
            }}
            className={selectCls}
          >
            <option value="all">All Action Types</option>
            {Object.entries(ACTION_TYPE_LABELS).map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        </div>
      </div>
    </div>
  );
}

