"use client";

import dynamic from "next/dynamic";

import { MUMBAI_AREAS, matchArea } from "@/lib/mumbai-areas";
import type { DimCount, Filters } from "@/lib/types";

const LeafletMap = dynamic(
  () => import("./leaflet-map").then((m) => m.LeafletMap),
  {
    ssr: false,
    loading: () => (
      <div className="h-80 w-full animate-pulse rounded-lg bg-muted/40" />
    ),
  },
);

export type MapPoint = { key: string; n: number; lat: number; lng: number };

export function AreaMap({
  data,
  filters,
}: {
  data: DimCount[];
  filters: Filters;
}) {
  const counts = new Map<string, number>();
  for (const d of data) {
    const key = matchArea(d.key);
    if (key) counts.set(key, (counts.get(key) ?? 0) + d.n);
  }
  const points: MapPoint[] = [...counts.entries()]
    .filter(([key]) => MUMBAI_AREAS[key])
    .map(([key, n]) => ({ key, n, ...MUMBAI_AREAS[key] }))
    .sort((a, b) => b.n - a.n);

  return (
    <div className="flex flex-col rounded-xl border border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
      <div className="flex items-center justify-between gap-2 border-b border-border/50 pb-2">
        <div>
          <div className="font-mono text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
            {"// AREA MAP"}
          </div>
          <div className="text-sm font-semibold tracking-tight text-foreground">
            Mumbai areas
          </div>
        </div>
        <span className="rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
          {points.length} areas
        </span>
      </div>
      {points.length ? (
        <div className="mt-2">
          <LeafletMap points={points} filters={filters} />
        </div>
      ) : (
        <p className="py-8 text-center font-mono text-xs text-muted-foreground">
          No mappable areas
        </p>
      )}
      <p className="mt-1 font-mono text-[10px] text-muted-foreground">
        Dot size ~ action count · click to filter
      </p>
    </div>
  );
}
