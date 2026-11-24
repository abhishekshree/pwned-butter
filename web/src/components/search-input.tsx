"use client";

import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import { useRouter } from "next/navigation";

import { buildHref } from "@/lib/href";
import type { Filters } from "@/lib/types";

export function SearchInput({
  initial,
  filters,
}: {
  initial: string;
  filters: Filters;
}) {
  const router = useRouter();
  const [q, setQ] = useState(initial);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const onChange = (value: string) => {
    setQ(value);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      router.replace(buildHref({ ...filters, q: value.trim() || undefined }));
    }, 300);
  };

  return (
    <div className="relative">
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <input
        type="search"
        name="q"
        autoComplete="off"
        value={q}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Search outlets, areas or brands…"
        aria-label="Search enforcement records"
        className="h-11 w-full rounded-lg border border-border bg-card pr-9 pl-9 text-sm text-foreground outline-none transition-colors placeholder:text-muted-foreground/70 hover:border-primary/50 focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring"
      />
      {q ? (
        <button
          type="button"
          onClick={() => onChange("")}
          aria-label="Clear search"
          className="absolute top-1/2 right-2.5 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      ) : null}
    </div>
  );
}
