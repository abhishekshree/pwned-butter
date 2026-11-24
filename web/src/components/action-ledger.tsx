import { ArrowUpRight, ChevronDown } from "lucide-react";

import {
  STATUS_CONFIG,
  actionTypeLabel,
  complianceTone,
  formatDate,
  humanize,
  outletTypeLabel,
} from "@/lib/format";
import type { ActionRow } from "@/lib/types";
import { cn } from "@/lib/utils";

const GRID = "md:grid-cols-[5.5rem_minmax(0,1.4fr)_8rem_6.5rem_7.5rem]";

function SourceLink({ a }: { a: ActionRow }) {
  return (
    <a
      href={a.source_url}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-flex min-w-0 items-center gap-1 truncate text-xs text-primary hover:underline focus-visible:ring-2 focus-visible:ring-ring md:justify-self-end"
    >
      <span className="truncate">{a.source_publisher ?? "News report"}</span>
      <ArrowUpRight className="size-3 shrink-0" aria-hidden="true" />
    </a>
  );
}

export function ActionLedger({ actions }: { actions: ActionRow[] }) {
  return (
    <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      <div
        className={cn(
          "hidden gap-3 border-b border-border bg-muted/50 px-4 py-2.5 text-[11px] font-medium text-muted-foreground md:grid",
          GRID,
        )}
      >
        <span>Date</span>
        <span>Outlet</span>
        <span>Where</span>
        <span>Status</span>
        <span>Source</span>
      </div>

      {actions.map((a) => {
        const loc = [a.area, a.city].filter(Boolean).join(", ");
        const violations = a.violations ?? [];
        const status = STATUS_CONFIG[a.status] ?? {
          label: humanize(a.status),
          tone: "text-muted-foreground",
        };
        const comp =
          a.compliance_score != null ? complianceTone(a.compliance_score) : null;

        return (
          <details
            key={a.id}
            className="group/row px-4 py-3 transition-colors hover:bg-muted/30 open:bg-muted/20"
          >
            <summary
              className={cn(
                "grid cursor-pointer grid-cols-[minmax(0,1fr)_auto] list-none gap-x-3 gap-y-1 rounded-sm [&::-webkit-details-marker]:hidden md:items-center md:gap-3",
                GRID,
              )}
            >
              <span className="hidden text-xs tabular-nums text-muted-foreground md:block">
                {formatDate(a.action_date)}
              </span>

              <span className="min-w-0">
                <span className="block truncate text-sm font-medium text-foreground">
                  {a.establishment}
                </span>
                {a.brand &&
                a.brand.toLowerCase() !== a.establishment.toLowerCase() ? (
                  <span className="block truncate text-xs text-muted-foreground">
                    {a.brand}
                  </span>
                ) : null}
              </span>

              <span className="hidden truncate text-xs text-muted-foreground md:block" title={loc}>
                {loc || "—"}
              </span>

              <span className="flex items-center justify-end gap-2 md:justify-start">
                <span className={cn("text-xs font-medium", status.tone)}>
                  {status.label}
                </span>
                <ChevronDown className="size-3.5 shrink-0 text-muted-foreground transition-transform duration-150 group-open/row:rotate-180 md:hidden" />
              </span>

              <span className="col-span-2 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground md:hidden">
                <span className="tabular-nums">{formatDate(a.action_date)}</span>
                {loc ? <span className="truncate">{loc}</span> : null}
              </span>

              <span className="hidden md:block">
                <SourceLink a={a} />
              </span>
            </summary>

            <div className="mt-3 space-y-2 border-t border-border pt-3 text-sm md:pl-[6.25rem]">
              <p className="text-xs font-medium text-foreground">
                {actionTypeLabel(a.action_type)}
                {a.outlet_type && a.outlet_type !== "other" ? (
                  <span className="ml-2 font-normal text-muted-foreground">
                    {outletTypeLabel(a.outlet_type)}
                  </span>
                ) : null}
              </p>

              {a.details ? (
                <p className="max-w-3xl text-[13px] leading-relaxed text-foreground/80">
                  {a.details}
                </p>
              ) : null}

              {violations.length ? (
                <ul className="max-w-3xl space-y-1">
                  {violations.map((v) => (
                    <li
                      key={v}
                      className="flex gap-2 text-xs leading-snug text-muted-foreground"
                    >
                      <span className="mt-1.5 size-1 shrink-0 rounded-full bg-status-suspended" />
                      <span>{v}</span>
                    </li>
                  ))}
                </ul>
              ) : null}

              {a.platforms?.length || comp ? (
                <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  {a.platforms?.length ? (
                    <span>Listed on {a.platforms.map(humanize).join(", ")}</span>
                  ) : null}
                  {comp && a.compliance_score != null ? (
                    <span className={cn("font-medium", comp)}>
                      Compliance {a.compliance_score}%
                    </span>
                  ) : null}
                </div>
              ) : null}

              <div className="md:hidden">
                <SourceLink a={a} />
              </div>
            </div>
          </details>
        );
      })}
    </div>
  );
}
