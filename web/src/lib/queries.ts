import { unstable_cache } from "next/cache";

import { sql } from "./db";
import { activeFilter } from "./href";
import type { ActionRow, DimCount, Filters, RunSummary, Stats } from "./types";

const CACHE = { revalidate: 1800 };

const toInt = (v: unknown): number =>
  typeof v === "string" ? parseInt(v, 10) : Number(v ?? 0);

async function queryRows<T extends Record<string, unknown>>(text: string): Promise<T[]> {
  return (await sql(text)) as T[];
}

export const listActions = unstable_cache(
  async (
    f: Filters,
    opts: { limit: number; offset: number },
  ): Promise<{ total: number; actions: ActionRow[] }> => {
    const params: unknown[] = [];
    const conds: string[] = [];

    const eq = (col: string, v: string | undefined) => {
      const val = activeFilter(v);
      if (val) {
        params.push(val);
        conds.push(`${col} = $${params.length}`);
      }
    };

    eq("brand", f.brand);
    eq("city", f.city);
    eq("status", f.status);
    eq("action_type", f.action_type);
    eq("outlet_type", f.outlet_type);

    const q = f.q?.trim();
    if (q) {
      const like = `%${q}%`;
      params.push(like, like, like);
      conds.push(
        `(establishment ILIKE $${params.length - 2} OR brand ILIKE $${params.length - 1} OR area ILIKE $${params.length})`,
      );
    }
    if (f.from) {
      params.push(f.from);
      conds.push(`action_date >= $${params.length}`);
    }
    if (f.to) {
      params.push(f.to);
      conds.push(`action_date <= $${params.length}`);
    }

    const where = conds.length ? `WHERE ${conds.join(" AND ")}` : "WHERE true";
    const filterParams = [...params];
    const limit = Math.min(Math.max(opts.limit, 1), 200);
    const offset = Math.max(Math.floor(opts.offset), 0);
    const text = `SELECT actions.*, COUNT(*) OVER()::int AS total_count FROM actions ${where}
      ORDER BY action_date DESC, id DESC LIMIT $${filterParams.length + 1} OFFSET $${filterParams.length + 2}`;

    const rows = (await sql(text, [...filterParams, limit, offset])) as Array<
      Record<string, unknown>
    >;
    const total = rows[0]
      ? toInt(rows[0].total_count)
      : offset
        ? toInt(
            (
              (await sql(
                `SELECT COUNT(*)::int AS total_count FROM actions ${where}`,
                filterParams,
              )) as Array<Record<string, unknown>>
            )[0]?.total_count,
          )
        : 0;
    return {
      total,
      actions: rows as unknown as ActionRow[],
    };
  },
  ["list-actions", "v3"],
  CACHE,
);

async function dims(col: string, limit?: number): Promise<DimCount[]> {
  const query = `SELECT COALESCE(NULLIF(${col}, ''), 'unknown') AS k, COUNT(*)::int AS n
     FROM actions GROUP BY k ORDER BY n DESC${limit ? " LIMIT $1" : ""}`;
  const rows = (await sql(
    query,
    limit ? [limit] : [],
  )) as Array<Record<string, unknown>>;
  return rows.map((r) => ({
    key: String(r.k),
    n: toInt(r.n),
  }));
}

function normalizeRun(r: Record<string, unknown>): RunSummary {
  return {
    id: toInt(r.id),
    started_at: String(r.started_at ?? ""),
    finished_at: r.finished_at ? String(r.finished_at) : null,
    status: String(r.status ?? ""),
    articles_seen: toInt(r.articles_seen),
    articles_new: toInt(r.articles_new),
    actions_upserted: toInt(r.actions_upserted),
    llm_calls: toInt(r.llm_calls),
    error: r.error ? String(r.error) : null,
  };
}

export const getStats = unstable_cache(
  async (): Promise<Stats> => {
    const [total, runs, byStatus, byActionType, byCity, byArea, range] = await Promise.all([
      queryRows<{ n: unknown }>("SELECT COUNT(*)::int AS n FROM actions"),
      queryRows<Record<string, unknown>>(`SELECT id::int AS id, started_at, finished_at, status,
            articles_seen::int AS articles_seen, articles_new::int AS articles_new,
            actions_upserted::int AS actions_upserted, llm_calls::int AS llm_calls, error
     FROM fetch_runs ORDER BY id DESC LIMIT 1`),
      dims("status"),
      dims("action_type"),
      dims("city", 8),
      dims("area", 50),
      queryRows<Record<string, unknown>>(
        "SELECT MIN(action_date)::text AS min_d, MAX(action_date)::text AS max_d, COUNT(DISTINCT action_date)::int AS days FROM actions",
      ),
    ]);

    return {
      totalActions: total[0] ? toInt(total[0].n) : 0,
      lastRun: runs[0] ? normalizeRun(runs[0]) : null,
      byStatus,
      byActionType,
      byCity,
      byArea,
      dateRange: range[0]?.max_d
        ? {
            from: String(range[0].min_d ?? ""),
            to: String(range[0].max_d ?? ""),
            days: toInt(range[0].days),
          }
        : null,
    };
  },
  ["get-stats", "v4"],
  CACHE,
);
