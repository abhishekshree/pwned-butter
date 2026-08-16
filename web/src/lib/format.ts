export const ACTION_TYPE_LABELS: Record<string, string> = {
  licence_suspension: "Licence suspension",
  stop_business: "Stop business",
  improvement_notice: "Improvement notice",
  sealing: "Sealing",
  seizure: "Seizure",
  inspection: "Inspection",
  reopened: "Reopened",
};

export const OUTLET_TYPE_LABELS: Record<string, string> = {
  restaurant: "Restaurant",
  cloud_kitchen: "Cloud kitchen",
  quick_commerce: "Quick commerce",
  warehouse: "Warehouse",
  dhaba: "Dhaba",
  hotel: "Hotel",
  bakery: "Bakery",
  club: "Club",
  mess: "Mess",
  dairy: "Dairy",
  street_vendor: "Street vendor",
  other: "Other",
};

export const STATUS_TONES: Record<string, string> = {
  suspended: "text-red-500 border-red-500/30 bg-red-500/10",
  active: "text-amber-500 border-amber-500/30 bg-amber-500/10",
  reopened: "text-emerald-500 border-emerald-500/30 bg-emerald-500/10",
};

export const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
];

export function actionTypeLabel(v: string | null): string {
  if (!v) return "Unknown";
  return ACTION_TYPE_LABELS[v] ?? humanize(v);
}

export function outletTypeLabel(v: string | null): string {
  if (!v) return "Unknown";
  return OUTLET_TYPE_LABELS[v] ?? humanize(v);
}

export function humanize(s: string): string {
  return s.replace(/[_-]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export function formatDate(s: string | null | undefined): string {
  if (!s) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  return d.toLocaleDateString("en-IN", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

export function formatDateTime(s: string | null | undefined): string {
  if (!s) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  return d.toLocaleString("en-IN", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function complianceTone(score: number | null): string {
  if (score === null) return "text-muted-foreground border-border bg-muted/40";
  if (score >= 4) return "text-emerald-500 border-emerald-500/30 bg-emerald-500/10";
  if (score >= 3) return "text-amber-500 border-amber-500/30 bg-amber-500/10";
  return "text-red-500 border-red-500/30 bg-red-500/10";
}