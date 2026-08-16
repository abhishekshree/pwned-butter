import type { Filters } from "./types";

const EMPTY = /^(all|any)$/i;

export function activeFilter(v: string | undefined): string | undefined {
  const t = v?.trim();
  return t && !EMPTY.test(t) ? t : undefined;
}

export function buildHref(
  filters: Filters,
  overrides: { page?: number } = {},
): string {
  const params = new URLSearchParams();
  const set = (k: keyof Filters) => {
    const v = activeFilter(filters[k] as string | undefined);
    if (v) params.set(k, v);
  };
  set("brand");
  set("city");
  set("status");
  set("action_type");
  set("outlet_type");
  set("q");
  set("from");
  set("to");
  if (overrides.page && overrides.page > 1) {
    params.set("page", String(overrides.page));
  }
  const s = params.toString();
  return s ? `/?${s}` : "/";
}