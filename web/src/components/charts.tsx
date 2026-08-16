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
  payload?: { key?: unknown };
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
  return (
    <div className="rounded-lg border bg-background px-3 py-2 text-xs shadow-md">
      <div className="font-medium">{humanize(label)}</div>
      <div className="text-muted-foreground">
        {String(item.value)} action{item.value === "1" || item.value === 1 ? "" : "s"}
      </div>
    </div>
  );
}

const chartClass =
  "h-56 w-full text-xs [&_.recharts-cartesian-axis-tick_text]:fill-muted-foreground [&_.recharts-cartesian-grid_line[stroke='#ccc']]:stroke-border/50 [&_.recharts-curve.recharts-tooltip-cursor]:stroke-border [&_.recharts-layer]:outline-hidden [&_.recharts-polar-grid_[stroke='#ccc']]:stroke-border [&_.recharts-radial-bar-background-sector]:fill-muted [&_.recharts-rectangle.recharts-tooltip-cursor]:fill-muted [&_.recharts-sector]:outline-hidden [&_.recharts-sector[stroke='#fff']]:stroke-transparent [&_.recharts-surface]:outline-hidden";

function colorsFor(data: DimCount[]): Record<string, string> {
  return Object.fromEntries(
    data.map((d, i) => [d.key, CHART_COLORS[i % CHART_COLORS.length]]),
  );
}

export function DonutChart({
  title,
  description,
  data,
}: {
  title: string;
  description?: string;
  data: DimCount[];
}) {
  const colors = colorsFor(data);
  return (
    <div className="flex flex-col gap-2">
      <div>
        <div className="text-sm font-medium">{title}</div>
        {description ? (
          <div className="text-xs text-muted-foreground">{description}</div>
        ) : null}
      </div>
      <div className={chartClass}>
        <ResponsiveContainer width="100%" height="100%">
          <PieChart>
            <Tooltip content={<ChartTooltip />} />
            <Pie
              data={data}
              dataKey="n"
              nameKey="key"
              innerRadius={48}
              outerRadius={76}
              paddingAngle={2}
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
  description,
  data,
}: {
  title: string;
  description?: string;
  data: DimCount[];
}) {
  const colors = colorsFor(data);
  return (
    <div className="flex flex-col gap-2">
      <div>
        <div className="text-sm font-medium">{title}</div>
        {description ? (
          <div className="text-xs text-muted-foreground">{description}</div>
        ) : null}
      </div>
      <div className={chartClass}>
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} layout="vertical" margin={{ left: 0, right: 8 }}>
            <XAxis type="number" hide />
            <YAxis
              type="category"
              dataKey="key"
              width={86}
              tickLine={false}
              axisLine={false}
              tick={{ fontSize: 11 }}
              tickFormatter={(v: string) => humanize(v)}
            />
            <Tooltip
              content={<ChartTooltip />}
              cursor={{ fill: "rgba(128,128,128,0.08)" }}
            />
            <Bar dataKey="n" radius={4} barSize={14}>
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
