export type ActionRow = {
  id: number;
  establishment: string;
  area: string | null;
  city: string | null;
  state: string;
  brand: string | null;
  operator: string | null;
  outlet_type: string | null;
  action_type: string;
  action_date: string;
  status: string;
  violations: string[];
  compliance_score: number | null;
  fssai_number: string | null;
  details: string | null;
  platforms: string[];
  source_url: string;
  source_publisher: string | null;
  source_headline: string | null;
  published_at: string | null;
  created_at: string;
  updated_at: string;
};

export type DimCount = { key: string; n: number };

export type RunSummary = {
  id: number;
  started_at: string;
  finished_at: string | null;
  status: string;
  articles_seen: number;
  articles_new: number;
  actions_upserted: number;
  llm_calls: number;
  error: string | null;
};

export type Stats = {
  totalActions: number;
  latestActionDate: string | null;
  lastRun: RunSummary | null;
  byStatus: DimCount[];
  byActionType: DimCount[];
  byCity: DimCount[];
  byBrand: DimCount[];
};

export type Filters = {
  brand?: string;
  city?: string;
  status?: string;
  action_type?: string;
  outlet_type?: string;
  q?: string;
  from?: string;
  to?: string;
};