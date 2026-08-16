# pwned-butter

Is the Mumbai butter real? Live tracker of Maharashtra FDA food-safety
enforcement — licence suspensions, stop-business orders, seals, seizures and
re-openings at restaurants, hotels, dhabas and quick-commerce dark stores.

The FDA publishes no machine-readable list. Records are built daily from
Google News RSS: a scheduled job reads the feeds, Gemini Flash extracts
structured entries, and each lands in Postgres with a link to its source
article.

## How it works

```
GitHub Actions (daily 06:00 UTC) ──► local_scrape (Rust)
  1. targeted Google News RSS queries (when:1d)
  2. follow redirects, dedupe, extract article text
  3. Gemini Flash extracts structured records (strict JSON)
  4. upsert into Neon, dedup on (source_url, establishment, action_date)
                │
Next.js dashboard (Vercel) ◄── reads Neon over HTTP (no lambdas)
```

Every run is logged to `fetch_runs` (articles seen, actions upserted, LLM
calls), so the pipeline is auditable.

## Tech

| Concern    | Choice                                             |
| ---------- | -------------------------------------------------- |
| ETL        | Rust (edition 2021)                                |
| Schedule   | GitHub Actions, daily 06:00 UTC                    |
| Hosting    | Next.js on Vercel (root directory `web/`)          |
| Database   | Neon Postgres (HTTP serverless driver)             |
| Extraction | Gemini Flash, free tier, backoff on 429            |
| News       | Google News RSS, no API key                        |
| Frontend   | Next.js App Router, Tailwind, shadcn/ui, Recharts  |
| Cost       | $0                                                 |

## Layout

```
src/news.rs              Google News RSS fetch, dedupe, article text
src/llm.rs               Gemini extraction + validation
src/db.rs                SQLx (Neon) writes + run bookkeeping
src/scrape.rs            daily pipeline orchestration
src/bin/local_scrape.rs  run the pipeline locally
src/bin/backfill.rs      day-by-day historical backfill
src/bin/migrate.rs       apply SQL migrations
migrations/              SQL schema
web/                     Next.js dashboard
.github/workflows/       CI + the daily scrape schedule
```

## Run locally

```bash
cp .env.example .env                # fill DATABASE_URL + GEMINI_API_KEY
cargo run --bin migrate             # apply schema
cargo run --release --bin local_scrape

cd web && pnpm install && pnpm dev   # dashboard (needs DATABASE_URL in web/.env.local)
```

Migrations only ever run explicitly via `migrate`; the dashboard never runs
them and reads over Neon's HTTP driver, which keeps serverless cold starts
fast.

## Deploy

1. Import in Vercel with root directory `web` (Framework preset: Next.js).
2. Set `DATABASE_URL` (Neon connection string).
3. Set repo secrets for the scrape workflow: `DATABASE_URL`,
   `GEMINI_API_KEY`.

The dashboard is URL-driven: every filter combination is a shareable link,
e.g. `/?brand=Domino's&city=Mumbai&status=suspended`.

## Caveats

- Suspension is not cancellation. Outlets often reopen after re-inspection;
  that is tracked as a separate `reopened` record.
- Some publishers block automated fetchers, so article text is not always
  available. Enrichment then falls back to headline + RSS data.
- This is a news-based compilation, not an official FDA register. Every
  record links to its source article.

## License

MIT
