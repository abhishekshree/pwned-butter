export const ACTION_TYPE_LABELS: Record<string, string> = {
  licence_suspension: "Licence Suspension",
  stop_business: "Stop Business Order",
  improvement_notice: "Improvement Notice",
  sealing: "Premises Sealed",
  seizure: "Seizure of Goods",
  inspection: "FDA Inspection",
  reopened: "Reopened / Cleared",
};

export const OUTLET_TYPE_LABELS: Record<string, string> = {
  restaurant: "Restaurant",
  cloud_kitchen: "Cloud Kitchen",
  quick_commerce: "Quick Commerce Dark Store",
  warehouse: "Food Warehouse",
  dhaba: "Dhaba / Eatery",
  hotel: "Hotel / Dining",
  bakery: "Bakery & Confectionery",
  club: "Club & Lounge",
  mess: "Mess / Canteen",
  dairy: "Dairy & Milk Center",
  street_vendor: "Street Food Vendor",
  other: "Food Establishment",
};

export const STATUS_CONFIG: Record<
  string,
  { label: string; tone: string; dot: string; edge: string; glow: string }
> = {
  suspended: {
    label: "SUSPENDED",
    tone: "text-red-600 dark:text-red-400 border-red-500/30 bg-red-500/10",
    dot: "bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.6)]",
    edge: "border-l-red-500 hover:border-l-red-400",
    glow: "group-hover:shadow-[0_0_20px_-5px_rgba(239,68,68,0.2)]",
  },
  reopened: {
    label: "REOPENED",
    tone: "text-emerald-600 dark:text-emerald-400 border-emerald-500/30 bg-emerald-500/10",
    dot: "bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.6)]",
    edge: "border-l-emerald-500 hover:border-l-emerald-400",
    glow: "group-hover:shadow-[0_0_20px_-5px_rgba(16,185,129,0.2)]",
  },
  active: {
    label: "NOTICE / RAID",
    tone: "text-amber-600 dark:text-amber-400 border-amber-500/30 bg-amber-500/10",
    dot: "bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.6)]",
    edge: "border-l-amber-500 hover:border-l-amber-400",
    glow: "group-hover:shadow-[0_0_20px_-5px_rgba(245,158,11,0.2)]",
  },
};

export const STATUS_TONES: Record<string, string> = {
  suspended: "text-red-600 dark:text-red-400 border-red-500/30 bg-red-500/10",
  active: "text-amber-600 dark:text-amber-400 border-amber-500/30 bg-amber-500/10",
  reopened: "text-emerald-600 dark:text-emerald-400 border-emerald-500/30 bg-emerald-500/10",
};

export const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
];

export function actionTypeLabel(v: string | null): string {
  if (!v) return "Action Logged";
  return ACTION_TYPE_LABELS[v] ?? humanize(v);
}

export function outletTypeLabel(v: string | null): string {
  if (!v) return "Establishment";
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
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatNumber(n: number | undefined | null): string {
  if (n == null) return "0";
  return new Intl.NumberFormat("en-IN").format(n);
}

export function complianceTone(score: number | null): { text: string; badge: string; border: string; bg: string } {
  if (score === null) {
    return {
      text: "text-muted-foreground",
      badge: "border-border bg-muted/30 text-muted-foreground",
      border: "border-border",
      bg: "bg-muted",
    };
  }
  if (score >= 80 || score >= 4) {
    return {
      text: "text-emerald-500",
      badge: "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
      border: "border-emerald-500/40",
      bg: "bg-emerald-500",
    };
  }
  if (score >= 60 || score >= 3) {
    return {
      text: "text-amber-500",
      badge: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
      border: "border-amber-500/40",
      bg: "bg-amber-500",
    };
  }
  return {
    text: "text-red-500",
    badge: "border-red-500/30 bg-red-500/10 text-red-600 dark:text-red-400",
    border: "border-red-500/40",
    bg: "bg-red-500",
  };
}