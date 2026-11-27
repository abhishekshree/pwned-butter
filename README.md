# pwned-butter

![CI: fmt, clippy, nextest](https://img.shields.io/github/actions/workflow/status/abhishekshree/pwned-butter/ci.yml?label=CI&logo=github)
![Daily scrape](https://img.shields.io/github/actions/workflow/status/abhishekshree/pwned-butter/scrape.yml?label=daily%20scrape)
![License: MIT](https://img.shields.io/github/license/abhishekshree/pwned-butter)
![Language: Rust](https://img.shields.io/github/languages/top/abhishekshree/pwned-butter)

Is the Mumbai butter real? A live tracker of Maharashtra FDA food-safety
enforcement: licence suspensions, stop-business orders, seals, seizures and
re-openings at restaurants, hotels, dhabas and dark stores.

The FDA publishes no machine-readable list, so a scheduled job builds the
records daily from Google News RSS. Groq's Qwen model pulls structured entries
out of the articles and stores each one in Postgres with a link to its source.

## How it works

Every day at 06:00 UTC a GitHub Action runs a Rust job that reads targeted
Google News RSS searches over an overlapping 72-hour window, prioritises
explicit enforcement reports, extracts the article text, and has Qwen
turn it into structured records (establishment, action, date). A second LLM
pass groups records describing the same real-world event (re-reports by other
outlets with name variants) and keeps one. Records are upserted into Neon,
deduplicated on source article, establishment, event window and name heuristics.
Each run is logged, so the pipeline stays auditable.

The Next.js dashboard reads straight from Neon over HTTP. Every filter
combination is a shareable URL, e.g. `/?brand=Domino's&city=Mumbai&status=suspended`.

## Run locally

```bash
cp .env.example .env                # fill GROQ_API_KEY and DATABASE_URL
cargo run --bin migrate             # apply schema
cargo run --release --bin local_scrape

cd web && pnpm install && pnpm dev
```

To run the full pipeline without touching Postgres, set `LOCAL_WRITER_PATH` to
a JSONL file (e.g. `/tmp/fda-actions.jsonl`). RSS and the LLM still run; records
and run summaries are appended to that file and its `.runs.jsonl` sibling
instead of Neon.

`LLM_PROVIDER` selects the vendor (`openrouter` or `groq`); only that
vendor's key is required. Groq's free tier is the default.
Strict JSON-schema output is disabled for Groq because it rejects top-level
array schemas.

Tests (what CI runs) via [cargo-nextest](https://nexte.st):

```bash
cargo nextest run
```

## Caveats

- Suspension is not cancellation. Outlets often reopen after re-inspection,
  tracked as a separate `reopened` record.
- Some publishers block automated fetchers, so article text is not always
  available; enrichment then falls back to headline and RSS data.
- This is a news-based compilation, not an official FDA register. Every record
  links to its source article.

## License

MIT
