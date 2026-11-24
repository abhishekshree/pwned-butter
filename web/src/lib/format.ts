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

// Mirrors domain::Status and the status CASE in migrations/0001_init.sql.
export const STATUS_CONFIG: Record<string, { label: string; tone: string }> = {
  suspended: { label: "Suspended", tone: "text-status-suspended" },
  reopened: { label: "Reopened", tone: "text-status-reopened" },
  active: { label: "Notice or raid", tone: "text-status-notice" },
};

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

// Scores are percentages (0–100), set by the extractor from the source article.
export function complianceTone(score: number): string {
  if (score >= 75) return "text-status-reopened";
  if (score >= 50) return "text-status-notice";
  return "text-status-suspended";
}