// Approximate centroids for Mumbai-area matching. Area strings in the DB are
// free text, so matchArea() does a case-insensitive substring match and
// returns the canonical key, or null when nothing matches (never fails).
export const MUMBAI_AREAS: Record<string, { lat: number; lng: number }> = {
  Borivali: { lat: 19.23, lng: 72.86 },
  Kandivali: { lat: 19.2, lng: 72.85 },
  Malad: { lat: 19.19, lng: 72.85 },
  Andheri: { lat: 19.12, lng: 72.85 },
  Versova: { lat: 19.13, lng: 72.81 },
  Juhu: { lat: 19.11, lng: 72.83 },
  "Vile Parle": { lat: 19.1, lng: 72.845 },
  Santacruz: { lat: 19.08, lng: 72.84 },
  Powai: { lat: 19.12, lng: 72.91 },
  Bandra: { lat: 19.06, lng: 72.83 },
  Kurla: { lat: 19.08, lng: 72.88 },
  Ghatkopar: { lat: 19.08, lng: 72.91 },
  Chembur: { lat: 19.06, lng: 72.89 },
  Sion: { lat: 19.04, lng: 72.86 },
  Dharavi: { lat: 19.04, lng: 72.855 },
  Dadar: { lat: 19.02, lng: 72.84 },
  "Lower Parel": { lat: 18.99, lng: 72.83 },
  Parel: { lat: 19.0, lng: 72.84 },
  Worli: { lat: 18.99, lng: 72.82 },
  Mazgaon: { lat: 18.97, lng: 72.845 },
  "Nariman Point": { lat: 18.925, lng: 72.82 },
  Fort: { lat: 18.93, lng: 72.835 },
  Colaba: { lat: 18.92, lng: 72.83 },
  Mulund: { lat: 19.17, lng: 72.95 },
  Thane: { lat: 19.22, lng: 72.97 },
  Vashi: { lat: 19.08, lng: 73.0 },
  Nerul: { lat: 19.03, lng: 73.02 },
};

export function matchArea(area: string | null): string | null {
  if (!area) return null;
  const a = area.toLowerCase().trim();
  for (const key of Object.keys(MUMBAI_AREAS)) {
    const k = key.toLowerCase();
    if (a.includes(k) || k.includes(a)) return key;
  }
  return null;
}
