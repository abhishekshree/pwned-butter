# Maharashtra FDA Enforcement Tracker

A daily tracker for Maharashtra FDA food-safety enforcement actions — licence
suspensions, raids, stop-business orders, seals and seizures at restaurants,
dhabas, clubs, quick-commerce dark stores and more.

Built **entirely in Rust**, deployed free on **Vercel** (native Rust runtime),
stored in **Neon Postgres**, and fed daily by a **Google News RSS + Gemini Flash**
pipeline. No official FDA API exists, so records are compiled from Indian news
reports and cross-linked to their source articles.

![free](https://img.shields.io/badge/cost-$0-success)

## How it works

```
Vercel Cron (daily, 06:00 UTC)  ──►  /api/cron_scrape (Rust)
  1. Query Google News RSS (10 targeted searches, when:1d)
  2. Dedupe + follow redirects + extract article text
  3. Gemini Flash extracts structured records (strict JSON)
  4. Upsert into Neon (dedupe on source_url + establishment + date)
                │
    public/  ◄─►  /api/actions /api/stats /api/brands /api/cities
```

Every scrape is logged to `fetch_runs`, including articles seen, actions
upserted and LLM calls, so the pipeline is auditable.

## Tech

| Concern    | Choice                                    |
| ---------- | ----------------------------------------- |
| Language   | Rust (edition 2021)                       |
| Hosting    | Vercel — native Rust runtime (`/api/*.rs`)|
| Schedule   | [Vercel Cron Jobs](https://vercel.com/docs/cron-jobs) (Hobby: once/day) |
| Database   | Neon Postgres (serverless)                |
| Extraction | Gemini Flash (`gemini-flash-latest`), free tier, retry-with-backoff on 429 |
| News       | Google News RSS (no API key)              |
| Frontend   | Static HTML/JS in `public/` (no build step) |

## Project layout

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

## Run the scraper locally

```bash
cp .env.example .env   # fill DATABASE_URL + GEMINI_API_KEY
cargo run --release --bin local_scrape
```

The migration applies automatically on first run.

## Schema

`actions` — one row per establishment per action: name, area, city, brand,
operator, outlet type, action type (`licence_suspension`, `stop_business`,
`improvement_notice`, `sealing`, `seizure`, `inspection`, `reopened`), date,
status (derived in the DB from `action_type`: `suspended` / `reopened` /
`active`), violations (TEXT[]), compliance score, delivery platforms
(`zomato`, `swiggy`, `blinkit`, ...), and the source article.

`fetch_runs` — pipeline audit log.

## Deploy to Vercel

1. Push this repo to GitHub.
2. Import it in the Vercel dashboard (Framework preset: *Other*).
3. Add env vars: `DATABASE_URL`, `GEMINI_API_KEY`, `CRON_SECRET`.
   - `DATABASE_URL`: your Neon pooled connection string.
   - `GEMINI_API_KEY`: from [aistudio.google.com](https://aistudio.google.com) (free). Check
     live rate limits for your project — build `429` retries into mind, not code.
   - `CRON_SECRET`: a random string used to trigger `/api/cron_scrape` manually.
4. The cron in `vercel.json` runs the scraper daily at 06:00 UTC on Hobby.

`vercel dev` will run a local Rust/Rust dev server; `vercel deploy` for production.

## API examples

```
GET /api/actions?brand=Domino's&city=Mumbai&status=suspended
GET /api/actions?q=dhaba&from=2026-08-01&to=2026-08-15
GET /api/stats
GET /api/brands
```

## Caveats

- **Suspension ≠ cancellation.** Outlets frequently reopen after re-inspection
  (see `status: reopened`) — that's captured as separate `reopened` records.
- News-derived data: some publishers block automated fetchers, so enrichment
  falls back to headline + RSS data. Expect occasional gaps.
- `/api/cron_scrape` accepts Vercel cron calls by `User-Agent` (the only header
  Vercel crons can send) and manual calls via `CRON_SECRET`. When `CRON_SECRET`
  is unset, manual calls are rejected — only the daily cron runs the scraper.
- This is an unofficial, news-based compilation, not an official FDA register.
  Every record links to its source article; verify before relying on it.

## License

MIT