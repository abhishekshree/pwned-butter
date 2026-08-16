CREATE TABLE IF NOT EXISTS actions (
    id               BIGSERIAL PRIMARY KEY,
    establishment    TEXT NOT NULL,
    area             TEXT,
    city             TEXT,
    state            TEXT NOT NULL DEFAULT 'Maharashtra',
    brand            TEXT,
    operator         TEXT,
    outlet_type      TEXT,
    action_type      TEXT NOT NULL CHECK (action_type IN (
        'licence_suspension', 'stop_business', 'improvement_notice',
        'sealing', 'seizure', 'inspection', 'reopened'
    )),
    action_date      DATE NOT NULL,
    status           TEXT GENERATED ALWAYS AS (
        CASE action_type
            WHEN 'reopened' THEN 'reopened'
            WHEN 'improvement_notice' THEN 'active'
            WHEN 'inspection' THEN 'active'
            WHEN 'seizure' THEN 'active'
            ELSE 'suspended'
        END
    ) STORED,
    violations       TEXT[] NOT NULL DEFAULT '{}',
    compliance_score INTEGER,
    fssai_number     TEXT,
    details          TEXT,
    platforms        TEXT[] NOT NULL DEFAULT '{}',
    source_url       TEXT NOT NULL,
    source_publisher TEXT,
    source_headline  TEXT,
    published_at     TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_url, establishment, action_date)
);

CREATE INDEX IF NOT EXISTS idx_actions_brand ON actions (brand);
CREATE INDEX IF NOT EXISTS idx_actions_city ON actions (city);
CREATE INDEX IF NOT EXISTS idx_actions_status ON actions (status);
CREATE INDEX IF NOT EXISTS idx_actions_type ON actions (action_type);
CREATE INDEX IF NOT EXISTS idx_actions_date ON actions (action_date);

CREATE TABLE IF NOT EXISTS fetch_runs (
    id               BIGSERIAL PRIMARY KEY,
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at      TIMESTAMPTZ,
    status           TEXT NOT NULL DEFAULT 'running',
    articles_seen    BIGINT NOT NULL DEFAULT 0,
    articles_new     BIGINT NOT NULL DEFAULT 0,
    actions_upserted BIGINT NOT NULL DEFAULT 0,
    llm_calls        BIGINT NOT NULL DEFAULT 0,
    details          JSONB NOT NULL DEFAULT '{}'::jsonb,
    error            TEXT
);