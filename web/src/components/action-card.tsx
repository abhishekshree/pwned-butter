import {
  ArrowUpRight,
  Building2,
  Calendar,
  MapPin,
  Newspaper,
  ShieldAlert,
  ShoppingBag,
} from "lucide-react";

import {
  STATUS_CONFIG,
  actionTypeLabel,
  complianceTone,
  formatDate,
  outletTypeLabel,
} from "@/lib/format";
import type { ActionRow } from "@/lib/types";
import { cn } from "@/lib/utils";

const PLATFORM_STYLES: Record<string, string> = {
  zomato: "border-red-500/30 bg-red-500/10 text-red-600 dark:text-red-400",
  swiggy: "border-orange-500/30 bg-orange-500/10 text-orange-600 dark:text-orange-400",
  blinkit: "border-yellow-500/30 bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
  zepto: "border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-400",
  instamart: "border-orange-500/30 bg-orange-500/10 text-orange-600 dark:text-orange-400",
};

export function ActionCard({ a }: { a: ActionRow }) {
  const loc = [a.area, a.city].filter(Boolean).join(", ");
  const violations = a.violations ?? [];
  const statusCfg = STATUS_CONFIG[a.status] ?? {
    label: a.status.toUpperCase(),
    tone: "border-border bg-muted/40 text-muted-foreground",
    dot: "bg-muted-foreground",
    edge: "border-l-border",
    glow: "",
  };
  const comp = a.compliance_score != null ? complianceTone(a.compliance_score) : null;

  return (
    <article
      className={cn(
        "group relative flex h-full flex-col justify-between rounded-lg border border-border/80 bg-card p-3 sm:p-3.5 text-card-foreground shadow-2xs transition-all duration-150 hover:-translate-y-0.5 hover:border-border hover:shadow-xs dark:bg-card/90",
        "border-l-[3px]",
        statusCfg.edge,
        statusCfg.glow,
      )}
    >
      <div className="space-y-2">
        {/* Compact Header: Date + Action + Status */}
        <div className="flex flex-wrap items-center justify-between gap-1.5">
          <div className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
            <Calendar className="size-3 text-muted-foreground/70" />
            <span>{formatDate(a.action_date)}</span>
          </div>

          <div className="flex items-center gap-1.5">
            {a.action_type ? (
              <span className="inline-flex items-center rounded border border-border/70 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
                {actionTypeLabel(a.action_type)}
              </span>
            ) : null}

            <span
              className={cn(
                "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[10px] font-semibold tracking-wide",
                statusCfg.tone,
              )}
            >
              <span className={cn("size-1.5 rounded-full", statusCfg.dot)} />
              {statusCfg.label}
            </span>
          </div>
        </div>

        {/* Establishment name + Location & Brand in compact inline block */}
        <div>
          <div className="flex flex-wrap items-baseline justify-between gap-x-2 gap-y-0.5">
            <div className="flex flex-wrap items-baseline gap-1.5">
              <h2 className="text-sm sm:text-[15px] font-bold tracking-tight text-foreground transition-colors group-hover:text-primary">
                {a.establishment}
              </h2>
              {a.brand && a.brand.toLowerCase() !== a.establishment.toLowerCase() ? (
                <span className="inline-flex items-center rounded border border-primary/20 bg-primary/5 px-1.5 py-0.2 font-mono text-[10px] font-medium text-primary">
                  {a.brand}
                </span>
              ) : null}
            </div>

            {loc ? (
              <span className="inline-flex items-center gap-1 text-xs text-muted-foreground font-medium">
                <MapPin className="size-3 text-muted-foreground/70" />
                {loc}
              </span>
            ) : null}
          </div>

          {a.outlet_type && a.outlet_type !== "other" ? (
            <div className="mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground">
              <Building2 className="size-2.5 text-muted-foreground/70" />
              <span>{outletTypeLabel(a.outlet_type)}</span>
            </div>
          ) : null}
        </div>

        {/* Action Details (Clamped if long) */}
        {a.details ? (
          <p className="text-xs leading-relaxed text-muted-foreground/90 line-clamp-2">
            {a.details}
          </p>
        ) : null}

        {/* Violations Callout (Compact) */}
        {violations.length ? (
          <div className="rounded-md border border-amber-500/20 bg-amber-500/5 px-2.5 py-1.5">
            <div className="flex items-center gap-1 font-mono text-[10px] font-semibold text-amber-600 dark:text-amber-400">
              <ShieldAlert className="size-3 shrink-0" />
              <span>VIOLATIONS ({violations.length})</span>
            </div>
            <ul className="mt-1 space-y-0.5">
              {violations.slice(0, 2).map((v) => (
                <li
                  key={v}
                  className="flex items-start gap-1.5 text-[11px] leading-snug text-foreground/85"
                >
                  <span className="mt-1 size-1 shrink-0 rounded-full bg-amber-500" />
                  <span className="line-clamp-1">{v}</span>
                </li>
              ))}
              {violations.length > 2 ? (
                <li className="font-mono text-[10px] text-muted-foreground">
                  +{violations.length - 2} more violations cited
                </li>
              ) : null}
            </ul>
          </div>
        ) : null}

        {/* Badges row: Platforms & Compliance */}
        <div className="flex flex-wrap items-center justify-between gap-1.5 pt-0.5">
          {a.platforms?.length ? (
            <div className="flex flex-wrap items-center gap-1">
              <span className="inline-flex items-center gap-1 font-mono text-[9px] uppercase text-muted-foreground mr-0.5">
                <ShoppingBag className="size-2.5" />
                Listed:
              </span>
              {a.platforms.map((p) => {
                const pKey = p.toLowerCase();
                const pStyle =
                  PLATFORM_STYLES[pKey] ??
                  "border-border bg-muted/50 text-muted-foreground";
                return (
                  <span
                    key={p}
                    className={cn(
                      "inline-flex items-center rounded border px-1 py-0.2 font-mono text-[9px] font-medium uppercase",
                      pStyle,
                    )}
                  >
                    {p}
                  </span>
                );
              })}
            </div>
          ) : <div />}

          {comp && a.compliance_score != null ? (
            <div
              className={cn(
                "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[10px] font-semibold",
                comp.badge,
              )}
            >
              <span>COMPLIANCE:</span>
              <span>{a.compliance_score}%</span>
            </div>
          ) : null}
        </div>
      </div>

      {/* Card Footer: Source & Link */}
      <div className="mt-2.5 flex items-center justify-between gap-2 border-t border-border/50 pt-2 text-[11px]">
        <div className="flex items-center gap-1 text-muted-foreground">
          <Newspaper className="size-3 text-muted-foreground/70" />
          <span className="truncate font-mono text-[10px]">
            {a.source_publisher ? `Source: ${a.source_publisher}` : "News Report"}
          </span>
        </div>

        <a
          href={a.source_url}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-0.5 font-mono text-[11px] font-semibold text-primary transition-colors hover:text-primary/80 group-hover:underline"
        >
          <span>View Report</span>
          <ArrowUpRight className="size-3 transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
        </a>
      </div>
    </article>
  );
}


