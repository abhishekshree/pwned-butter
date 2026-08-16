import { ArrowUpRight } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  STATUS_TONES,
  humanize,
  outletTypeLabel,
} from "@/lib/format";
import type { ActionRow } from "@/lib/types";
import { cn } from "@/lib/utils";

const EDGE_TONES: Record<string, string> = {
  suspended: "border-l-red-500",
  reopened: "border-l-emerald-500",
  active: "border-l-amber-500",
};

const KNOWN_PLATFORMS = new Set(["zomato", "swiggy", "blinkit", "zepto", "instamart"]);

export function ActionCard({ a }: { a: ActionRow }) {
  const loc = [a.area, a.city].filter(Boolean).join(" · ");
  const violations = a.violations ?? [];

  return (
    <a
      href={a.source_url}
      target="_blank"
      rel="noopener"
      className={cn(
        "group flex flex-col gap-2 rounded-2xl border border-l-4 bg-card p-4 text-card-foreground shadow-sm transition hover:-translate-y-px hover:shadow-md",
        EDGE_TONES[a.status] ?? "border-l-transparent",
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <time className="text-xs font-medium text-muted-foreground">{a.action_date}</time>
        <Badge variant="outline" className={STATUS_TONES[a.status] ?? "border-border bg-muted/40 text-muted-foreground"}>
          {humanize(a.status)}
        </Badge>
      </div>

      <h2 className="text-sm leading-snug font-medium text-foreground">{a.establishment}</h2>
      {loc ? <p className="-mt-1 text-xs text-muted-foreground">{loc}</p> : null}

      {(a.outlet_type && a.outlet_type !== "other") || (a.brand && a.brand !== a.establishment) ? (
        <div className="flex flex-wrap gap-1.5">
          {a.outlet_type && a.outlet_type !== "other" ? (
            <Badge variant="outline">{outletTypeLabel(a.outlet_type)}</Badge>
          ) : null}
          {a.brand && a.brand !== a.establishment ? <Badge variant="outline">{a.brand}</Badge> : null}
        </div>
      ) : null}

      {a.details ? <p className="text-sm text-muted-foreground">{a.details}</p> : null}

      {violations.length ? (
        <ul className="space-y-0.5">
          {violations.slice(0, 3).map((v) => (
            <li
              key={v}
              className="relative pl-4 text-xs text-muted-foreground before:absolute before:top-1 before:left-0 before:size-1.5 before:rounded-sm before:bg-amber-500"
            >
              {v}
            </li>
          ))}
          {violations.length > 3 ? (
            <li className="text-xs text-muted-foreground">+{violations.length - 3} more</li>
          ) : null}
        </ul>
      ) : null}

      {a.compliance_score != null ? (
        <div className="text-xs font-semibold text-amber-500">Compliance {a.compliance_score}%</div>
      ) : null}

      {a.platforms.length ? (
        <div className="flex flex-wrap gap-1">
          {a.platforms.map((p) => (
            <span
              key={p}
              className={cn(
                "rounded-full border px-2 py-0.5 text-[10px] font-medium",
                KNOWN_PLATFORMS.has(p)
                  ? "border-blue-500/30 text-blue-500"
                  : "border-border text-muted-foreground",
              )}
            >
              {p}
            </span>
          ))}
        </div>
      ) : null}

      <div className="mt-1 flex items-center justify-between gap-3 border-t border-dashed pt-2.5">
        <span className="flex items-center gap-1 text-xs font-semibold text-blue-500 group-hover:underline dark:text-blue-400">
          Read the report
          <ArrowUpRight className="size-3.5" />
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {a.source_publisher ? `via ${a.source_publisher}` : "article"}
        </span>
      </div>
    </a>
  );
}