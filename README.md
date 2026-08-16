# Maharashtra FDA Enforcement Tracker

Tracks food-safety enforcement actions by the Maharashtra FDA: licence
suspensions, stop-business orders, seals, seizures and re-openings at
restaurants, hotels, dhabas and quick-commerce dark stores.
Live at https://fda-mumbai-tracker.vercel.app

The FDA publishes no machine-readable list of these actions. Reports live in
newspapers instead, so this project builds the list by hand: a daily cron
reads Google News RSS, Gemini Flash extracts structured records from the
articles, and each row is stored in Postgres with a link to the article it
came from. It runs for free.

## How it works

```
Vercel Cron (daily 06:00 UTC)  ──►  /api/cron_scrape (Rust)
  1. 10 targeted Google News RSS queries (when:1d)
  2. follow redirects, dedupe, extract article text
  3. Gemini Flash extracts structured records (strict JSON)
  4. upsert into Neon, dedup on (source_url, establishment, action_date)
                │
    public/  ◄─►  /api/actions /api/stats /api/brands /api/cities
```

Every run is logged to `fetch_runs` with articles seen, actions upserted and
LLM calls, so the pipeline is auditable.

## Tech

| Concern    | Choice                                        |
| ---------- | --------------------------------------------- |
| Language   | Rust (edition 2021)                           |
| Hosting    | Vercel native Rust runtime (`/api/*.rs`)      |
| Schedule   | Vercel Cron, daily 06:00 UTC                  |
| Database   | Neon Postgres                                 |
| Extraction | Gemini Flash, free tier, backoff on 429       |
| News       | Google News RSS, no API key                   |
| Frontend   | Static HTML/JS in `public/`, no build step    |
| Cost       | $0                                            |

## Layout

```
api/cron_scrape.rs   daily scraper endpoint (Vercel cron)
api/actions.rs       filtered action list    GET /api/actions?brand=&city=&status=&q=&from=&to=
api/stats.rs         summary counts         GET /api/stats
api/brands.rs        brand → count          GET /api/brands
api/cities.rs        city → count           GET /api/cities
src/news.rs          Google News RSS fetch, dedupe, article text
src/llm.rs           Gemini extraction + validation
src/db.rs            SQLx (Neon) read/write + stats
src/scrape.rs        daily pipeline orchestration
src/bin/local_scrape.rs  run the pipeline locally
migrations/          SQL schema
public/              static dashboard frontend
```

## Run locally

```bash
cp .env.example .env                # fill DATABASE_URL + GEMINI_API_KEY
cargo run --bin migrate              # apply schema (only needed once / after changes)
cargo run --release --bin local_scrape
```

The API never runs migrations itself; that happens explicitly with `migrate`
so serverless cold starts don't fight over the migration lock.

## Deploy

1. Push to GitHub, import in Vercel (Framework preset: Other).
2. Set env vars: `DATABASE_URL`, `GEMINI_API_KEY`, `CRON_SECRET`.
   - `DATABASE_URL`: your Neon pooled connection string.
   - `GEMINI_API_KEY`: from aistudio.google.com, free.
   - `CRON_SECRET`: random string for manual triggers of `/api/cron_scrape`.
3. The cron in `vercel.json` fires daily at 06:00 UTC.

## API examples

```
GET /api/actions?brand=Domino's&city=Mumbai&status=suspended
GET /api/actions?q=dhaba&from=2026-08-01&to=2026-08-15
GET /api/stats
GET /api/brands
```

## Caveats

- Suspension is not cancellation. Outlets often reopen after re-inspection;
  that is tracked as a separate `reopened` record.
- Some publishers block automated fetchers, so article text is not always
  available. Enrichment then falls back to headline + RSS data.
- `/api/cron_scrape` accepts Vercel cron calls (matched by User-Agent, the
  only header Vercel crons can send) and manual calls via `CRON_SECRET`.
- This is a news-based compilation, not an official FDA register. Every record
  links to its source article.

## License

MIT
