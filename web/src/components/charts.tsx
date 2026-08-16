"use client";

import {
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { CHART_COLORS, humanize } from "@/lib/format";
import type { DimCount } from "@/lib/types";

type Payload = {
  value?: number | string;
  name?: string;
  payload?: { key?: unknown; n?: number };
};

function ChartTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: Payload[];
}) {
  if (!active || !payload?.length) return null;
  const item = payload[0];
  const label = String(item.payload?.key ?? item.name ?? "");
  const value = item.payload?.n ?? item.value ?? 0;
  return (
    <div className="rounded-lg border border-border/80 bg-background/95 p-2.5 shadow-lg backdrop-blur-md">
      <div className="text-xs font-semibold text-foreground">{humanize(label)}</div>
      <div className="mt-1 flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
        <span className="font-bold text-foreground">{value}</span>
        <span>action{Number(value) === 1 ? "" : "s"}</span>
      </div>
    </div>
  );
}

const chartClass =
  "h-52 w-full text-xs [&_.recharts-cartesian-axis-tick_text]:fill-muted-foreground [&_.recharts-cartesian-grid_line]:stroke-border/40 [&_.recharts-layer]:outline-hidden [&_.recharts-sector]:outline-hidden [&_.recharts-surface]:outline-hidden";

function colorsFor(data: DimCount[]): Record<string, string> {
  return Object.fromEntries(
    data.map((d, i) => [d.key, CHART_COLORS[i % CHART_COLORS.length]]),
  );
}

export function DonutChart({
  title,
  data,
  tag,
}: {
  title: string;
  data: DimCount[];
  tag?: string;
}) {
  const colors = colorsFor(data);
  const total = data.reduce((acc, d) => acc + (d.n || 0), 0);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2 border-b border-border/50 pb-2">
        <div>
          <div className="font-mono text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
            {tag ?? "// STATUS"}
          </div>
          <div className="text-sm font-semibold tracking-tight text-foreground">{title}</div>
        </div>
        <span className="rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
          {total} total
        </span>
      </div>
      <div className={chartClass}>
        <ResponsiveContainer width="100%" height="100%">
          <PieChart>
            <Tooltip content={<ChartTooltip />} />
            <Pie
              data={data}
              dataKey="n"
              nameKey="key"
              innerRadius={46}
              outerRadius={74}
              paddingAngle={3}
              strokeWidth={0}
            >
              {data.map((d) => (
                <Cell key={d.key} fill={colors[d.key]} />
              ))}
            </Pie>
          </PieChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

export function BarChartCard({
  title,
  data,
  tag,
}: {
  title: string;
  data: DimCount[];
  tag?: string;
}) {
  const colors = colorsFor(data);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2 border-b border-border/50 pb-2">
        <div>
          <div className="font-mono text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
            {tag ?? "// ANALYTICS"}
          </div>
          <div className="text-sm font-semibold tracking-tight text-foreground">{title}</div>
        </div>
        <span className="rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
          {data.length} buckets
        </span>
      </div>
      <div className={chartClass}>
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} layout="vertical" margin={{ left: -10, right: 12, top: 4, bottom: 4 }}>
            <XAxis type="number" hide />
            <YAxis
              type="category"
              dataKey="key"
              width={96}
              tickLine={false}
              axisLine={false}
              tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
              tickFormatter={(v: string) => humanize(v)}
            />
            <Tooltip
              content={<ChartTooltip />}
              cursor={{ fill: "rgba(128,128,128,0.06)" }}
            />
            <Bar dataKey="n" radius={[0, 4, 4, 0]} barSize={13}>
              {data.map((d) => (
                <Cell key={d.key} fill={colors[d.key]} />
              ))}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

