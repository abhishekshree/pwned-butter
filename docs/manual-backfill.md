# Manual backfill runbook (opencode-driven)

The `backfill` binary was removed. When the daily scrape leaves a gap
(LLM outage, quota exhaustion, dead fallback), backfill it with this
procedure: research the missed window with parallel agents, review the
insert table, then write it in one idempotent transaction. No secrets
are needed until the final write step.

## 0. Confirm the gap

Daily runs stay green while inserting zero, so check the numbers, not
the checkmarks:

```sql
SELECT id, started_at, status, articles_seen, articles_new,
       actions_upserted, llm_calls, error
FROM fetch_runs ORDER BY id DESC LIMIT 10;
SELECT COUNT(*), MAX(action_date), MAX(created_at) FROM actions;
```

Runs with `articles_seen > 0 AND actions_upserted = 0` day after day =
outage gap. The per-run Job Summary on the Actions page shows the same
`run ok: seen=X new=Y upserted=Z llm_calls=C triage_failures=T extraction_failures=E rejected=R`
line; the three failure counters (`triage_failures`, `extraction_failures`,
`rejected`) distinguish a clean empty run from a degraded one. Full stderr
is downloadable without admin rights:

```bash
J=$(gh api repos/abhishekshree/pwned-butter/actions/runs/<RUN_ID>/jobs --jq '.jobs[0].id')
gh api repos/abhishekshree/pwned-butter/actions/jobs/$J/logs > /tmp/run.log
```

## 1. Scope the window

Cover from the last good `action_date` through yesterday. Prefer
re-reports dated inside the window over guessing older action dates;
the `(source_url, establishment, action_date)` key plus the ±5-day
`actions_no_dupe_events` exclusion make re-runs safe.

## 2. Fan out research agents

Split the window into 3–4 story clusters (by drive/event, not by date)
and run one agent per cluster in parallel. Every agent gets the same
extraction contract:

- One record per **named retail food outlet** with a concrete action
  (restaurant, hotel, café, cloud kitchen, bakery, sweet shop, dhaba,
  mess, canteen, quick-commerce food store).
- **Skip**: manufacturers, processing plants, warehouses, wholesale/B2B,
  dairy plants, farms, pharma, cosmetics, non-Maharashtra, unnamed
  outlets, pending replies, plans, profiles.
- Fields: `establishment, area, city, brand, operator,
  outlet_type, action_type, action_date, violations[≤5], details
  (one sentence), platforms, source_url, source_publisher,
  source_headline, published_at`. `outlet_type` / `action_type` use the
  exact lowercase codes from `src/models.rs`.
- **Omit, don't guess**: unknown area/violations stay empty. Never mix
  facts across incidents. Corroborate specifics across same-incident
  coverage (roundups need 2+ outlets reporting the same drive).
- Return: JSON array + skip list with reasons + per-record confidence
  (high/medium). Seed each agent with 2–3 known article URLs plus
  headline searches for the rest.

## 3. Merge and dedupe

- Merge cross-cluster duplicates, keeping the richest fields
  (re-reported drives appear in several outlets' coverage).
- Collapse same-outlet re-reports within ±5 days into one row.
- Apply the live quality gate by hand: thresholds come from
  `src/rules.toml`, enforcement lives in `src/domain/mod.rs` — city
  required, details ≥ 20 chars, establishment ≤ 9 words, not the
  regulator, not a product, not quote-led.
- Thin rows (named + suspended + city, no outlet-level violations) pass
  the gate's letter but not its spirit — flag them, default to cutting.
- Watch for namesakes in different cities (e.g. two Radhakrishna
  Hotels): the DB dedupes on name + window, so only one survives
  unless the names are disambiguated.

## 4. Generate the SQL

One file, one transaction, idempotent:

```sql
BEGIN;
INSERT INTO actions (establishment, area, city, brand, operator,
  outlet_type, action_type, action_date, violations, compliance_score,
  fssai_number, details, platforms, source_url, source_publisher,
  source_headline, published_at)
VALUES ('Kyani & Co', 'Kalbadevi', 'Mumbai', NULL, NULL, 'restaurant',
  'licence_suspension', '2026-09-27',
  ARRAY['cats in food preparation areas','rat droppings'],
  NULL, NULL, 'FSSAI licence suspended ...', '{}',
  'https://…', 'Livemint', 'Headline…', '2026-09-29')
ON CONFLICT (source_url, establishment, action_date) DO NOTHING;
-- … one statement per row …
COMMIT;
```

Rules: escape `'` as `''`; empty arrays as `'{}'`; NULLs for
unreported optional fields; `published_at` as `YYYY-MM-DD`.

## 5. Execute (needs DB access — this is the only step that does)

`neonctl` with an authenticated account reaches the `fda` database;
no secret handling required:

```bash
export DATABASE_URL=$(neonctl connection-string \
  --project-id <project> --database-name fda \
  --role-name fda --pooled -o json)
uv run --with 'psycopg[binary]' python /tmp/run_backfill.py
```

where `/tmp/run_backfill.py` is:

```python
"""Executes /tmp/manual_backfill.sql in one transaction.
Unique hits are absorbed by ON CONFLICT DO NOTHING; exclusion-constraint
hits (same outlet + action within ±5d of an existing row) are skipped
per-row via savepoint and reported. Logs a fetch_runs provenance row."""
import json, os, re, sys
CONFLICT = "ON CONFLICT (source_url, establishment, action_date) DO NOTHING;"
def main() -> int:
    import psycopg
    from psycopg import errors as pg_errors
    dsn = os.environ.get("DATABASE_URL")
    if not dsn:
        print("DATABASE_URL is not set", file=sys.stderr)
        return 2
    sql = open("/tmp/manual_backfill.sql", encoding="utf-8").read()
    clean = "\n".join(l for l in sql.splitlines() if not l.strip().startswith("--"))
    parts = clean.split(CONFLICT)
    inserts = [p.strip() for p in parts[:-1] if p.strip()]
    if inserts and inserts[0].upper().startswith("BEGIN;"):
        inserts[0] = inserts[0][len("BEGIN;"):].strip()
    with psycopg.connect(dsn, connect_timeout=20) as conn:
        with conn.cursor() as cur:
            cur.execute("SELECT COUNT(*) FROM actions")
            before = cur.fetchone()[0]
            inserted = skipped_exact = 0
            skipped_fuzzy = []
            for stmt in inserts:
                try:
                    with conn.transaction():
                        cur.execute(stmt + CONFLICT)
                        if (cur.rowcount or 0) > 0:
                            inserted += 1
                        else:
                            skipped_exact += 1
                except pg_errors.ExclusionViolation:
                    m = re.search(r"\('([^']+)'", stmt)
                    skipped_fuzzy.append(m.group(1) if m else "?")
            cur.execute("SELECT COUNT(*) FROM actions")
            after = cur.fetchone()[0]
            cur.execute(
                "INSERT INTO fetch_runs (status, articles_seen, articles_new,"
                " actions_upserted, llm_calls, details, finished_at)"
                " VALUES ('ok', 0, 0, %s, 0, %s::jsonb, now())",
                (inserted, json.dumps({"source": "manual_backfill",
                    "rows_attempted": len(inserts), "rows_inserted": inserted,
                    "skipped_exact_dupes": skipped_exact,
                    "skipped_same_event": skipped_fuzzy})),
            )
        conn.commit()
    print(f"before={before} after={after} inserted={inserted} "
          f"exact_dupes={skipped_exact} same_event={skipped_fuzzy}")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
```

Expected output looks like:
`before=73 after=135 inserted=62 exact_dupes=0 same_event=[...]`.

## 6. Verify

```sql
SELECT COUNT(*), MAX(action_date), MAX(created_at) FROM actions;
```

Then open the dashboard and confirm the new cards render.

## Gotchas learned the hard way

- Green runs mean nothing: always read `actions_upserted` and the failure
  counters (`triage_failures`, `extraction_failures`, `rejected`) in the
  run-summary line, never the checkmark. The pipeline still has no
  zero-streak alert — an empty run with a low failure count can still be
  degraded, so check all three counters.
- The OpenRouter fallback 404s entirely when the account requires
  zero data retention (free endpoints offer none). Fix is at
  `openrouter.ai/settings/privacy`, not in code.
- Free OpenRouter models get delisted without warning. Re-verify the
  pinned IDs (`openrouter/free` and fallback
  `inclusionai/ling-3.2-flash:free` in `src/config.rs`) against
  `https://openrouter.ai/api/v1/models` before relying on them.
- Thin rows and cross-city namesakes are the two judgment calls that
  recur; when unsure, keep the row and let the exclusion constraint
  decide — it reports what it skips.
