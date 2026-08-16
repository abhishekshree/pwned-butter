# Maharashtra FDA Enforcement Tracker

Tracks food-safety enforcement actions by the Maharashtra FDA: licence
suspensions, stop-business orders, seals, seizures and re-openings at
restaurants, hotels, dhabas and quick-commerce dark stores.

The FDA publishes no machine-readable list of these actions. Reports live in
newspapers instead, so this project builds the list by hand: a daily scheduled
job reads Google News RSS, Gemini Flash extracts structured records from the
articles, and each row is stored in Postgres with a link to the article it
came from. It runs for free.

## How it works

```
GitHub Actions (daily 06:00 UTC)  ──►  local_scrape (Rust)
  1. ~24 targeted Google News RSS queries (when:1d)
  2. follow redirects, dedupe, extract article text
  3. Gemini Flash extracts structured records (strict JSON)
  4. upsert into Neon, dedup on (source_url, establishment, action_date)
                │
   Next.js dashboard (Vercel)  ◄──  queries Neon over HTTP (no lambdas)
```

Every run is logged to `fetch_runs` with articles seen, actions upserted and
LLM calls, so the pipeline is auditable.

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

The dashboard reads the database directly from server-rendered React
components via Neon's HTTP driver. There are no API functions or serverless
DB pools, so there is no cold-start timeouts to tune and nothing to scale.

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
web/                     Next.js dashboard (shadcn/ui + Recharts)
.github/workflows/       CI + the daily scrape schedule
```

## Run locally

```bash
cp .env.example .env                # fill DATABASE_URL + GEMINI_API_KEY
cargo run --bin migrate             # apply schema (only needed once / after changes)
cargo run --release --bin local_scrape

# dashboard
cd web
npm install
npm run dev                        # needs DATABASE_URL in web/.env.local
```

Migrations only ever run explicitly via the `migrate` bin. The dashboard never
runs them, and reads go over Neon's HTTP driver instead of a pooled TCP
connection, which is what keeps serverless cold starts fast and reliable.

## Deploy

1. Push to GitHub; import in Vercel with **root directory** set to `web`
   (Framework preset: Next.js).
2. Set env vars:
   - `DATABASE_URL` (Neon connection string; the HTTP driver works with the
     pooled URL)
3. Set repository secrets for the scrape workflow:
   - `DATABASE_URL` (same as above)
   - `GEMINI_API_KEY` (from aistudio.google.com, free)
4. The scrape workflow in `.github/workflows/scrape.yml` runs daily at 06:00
   UTC and can be triggered manually from the Actions tab.

The dashboard is URL-driven: every filter combination is a shareable link,
e.g. `/?brand=Domino's&city=Mumbai&status=suspended`.

## Caveats

- Suspension is not cancellation. Outlets often reopen after re-inspection;
  that is tracked as a separate `reopened` record.
- Some publishers block automated fetchers, so article text is not always
  available. Enrichment then falls back to headline + RSS data.
- This is a news-based compilation, not an official FDA register. Every record
  links to its source article.

## License

MIT