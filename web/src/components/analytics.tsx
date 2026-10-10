"use client";

import { useState } from "react";
import { ChevronDown } from "lucide-react";

import { BarChartCard, DonutChart } from "@/components/charts";
import { Card } from "@/components/ui/card";
import type { DimCount } from "@/lib/types";
import { cn } from "@/lib/utils";

// Charts mount only when opened: Recharts measures its container, and inside
// a closed <details> that width is 0, which throws during render.
export function Analytics({
  byStatus,
  byActionType,
  byCity,
}: {
  byStatus: DimCount[];
  byActionType: DimCount[];
  byCity: DimCount[];
}) {
  const [open, setOpen] = useState(false);

  return (
    <div className="rounded-xl border border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center justify-between gap-2 text-sm font-semibold text-foreground"
      >
        <span>Analytics</span>
        <ChevronDown
          className={cn("size-4 text-muted-foreground transition-transform", open && "rotate-180")}
        />
      </button>
      {open ? (
        <div className="mt-3 grid grid-cols-1 gap-4 md:grid-cols-3">
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <DonutChart title="Actions by Status" data={byStatus} tag="// STATUS RATIO" />
          </Card>
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <BarChartCard title="Enforcement Actions" data={byActionType} tag="// ACTION TYPES" />
          </Card>
          <Card className="rounded-xl border-border/80 bg-card p-4 shadow-xs dark:bg-card/90">
            <BarChartCard title="Top Regional Centers" data={byCity} tag="// TOP CITIES" />
          </Card>
        </div>
      ) : null}
    </div>
  );
}
