"use client";

import { useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import { useRouter } from "next/navigation";

import { ACTION_TYPE_LABELS } from "@/lib/format";
import { buildHref } from "@/lib/href";
import type { Filters } from "@/lib/types";

const selectCls =
  "h-9 w-full rounded-lg border border-input bg-card px-2 text-sm text-foreground outline-none focus-visible:border-ring sm:w-auto dark:bg-input/30";

function selectValue(v: string | undefined, fallback: string): string {
  return v && /^(all|any)$/i.test(v) ? "all" : v ?? fallback;
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
    }, 350);
  };

  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-start">
      <div className="relative flex-1 sm:max-w-xs">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <input
          type="search"
          value={q}
          onChange={(e) => onSearch(e.target.value)}
          placeholder="Search establishment, brand, area…"
          className="h-11 w-full rounded-lg border border-input bg-card pr-3 pl-9 text-base text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring dark:bg-input/30"
        />
      </div>

      <div className="flex flex-wrap gap-2">
        <select
          aria-label="Filter by brand"
          value={brand}
          onChange={(e) => {
            setBrand(e.target.value);
            onSelect("brand", e.target.value);
          }}
          className={selectCls}
        >
          <option value="all">All brands</option>
          {brands.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>

        <select
          aria-label="Filter by city"
          value={city}
          onChange={(e) => {
            setCity(e.target.value);
            onSelect("city", e.target.value);
          }}
          className={selectCls}
        >
          <option value="all">All cities</option>
          {cities.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>

        <select
          aria-label="Filter by action"
          value={actionType}
          onChange={(e) => {
            setActionType(e.target.value);
            onSelect("action_type", e.target.value);
          }}
          className={selectCls}
        >
          <option value="all">All actions</option>
          {Object.entries(ACTION_TYPE_LABELS).map(([v, label]) => (
            <option key={v} value={v}>
              {label}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
