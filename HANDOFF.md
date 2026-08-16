# HANDOFF — Maharashtra FDA Enforcement Tracker

State as of 2026-08-16 (post-refactor: build green, repo published). Read this first when picking up the project.

## What this is

A public GitHub repo → Vercel app that tracks Maharashtra FDA food-safety enforcement
(licence suspensions, raids, seals, quick-commerce raids) from Indian news. Daily cron
pulls Google News RSS, Gemini Flash extracts structured records, upserts into Neon
Postgres, and a static dashboard + Rust API serves it. Target cost: $0 (Vercel Hobby,
Neon free tier, Gemini free tier).

## Decisions already made (do not revisit)

- **Stack**: All-Rust. Native Vercel Rust runtime (`api/*.rs` bins, `vercel_runtime`
  crate v2). Confirmed against Vercel docs.
- **LLM**: Gemini Flash (`gemini-2.5-flash` via aistudio free key). Free tier exists but
  rate limits are per-project and non-public → the code has retry-with-backoff on `429`.
- **Frontend**: minimal static dashboard (plain HTML/JS in `public/`, no build step).
- **DB**: Neon Postgres, pooled connection string.
- **Cron**: Vercel cron in `vercel.json`, daily 06:00 UTC (Hobby allows 1x/day).
- **Attribution**: every row keeps source URL/publisher/headline; README carries a
  news-derived disclaimer. Suspension ≠ cancellation; status `reopened` tracks reopenings.

## Hard blockers — the tree DOES NOT COMPILE (9 errors)

Root cause of most: `Cargo.toml` sets `default-features = false` on `sqlx` without
re-enabling the features the code needs. Exact list (location → cause → fix):

| Error | Location | Fix |
|---|---|---|
| E0433 `sqlx::migrate!` unresolved | `src/db.rs:13` | add sqlx feature `macros` |
| E0433 `derive FromRow` not found | `src/db.rs:37` | same (`macros`) |
| E0432 `unresolved import sqlx::query::QueryBuilder` | `src/db.rs:10` | add sqlx feature `query-builder` |
| E0433 `cannot find macro json` ×6 | `src/db.rs:287,300,319` | `use serde_json::json;` |
| E0282 array of two `&mut QueryBuilder` can't infer type | `src/db.rs:206` | the `for qb in [&mut count_qb, &mut sel_qb]` loop won't unify lifetimes → duplicate block or drop count query entirely (see §1) |
| E0599 `rss::Source` has no `.value()` | `src/news.rs:65` | rss 2.1 `Source` is `{ url: String, title: Option<String> }` → `.title()` |
| E0277 `?` on a Future | `src/scrape.rs:50` | `news::build_client()` is async → add `.await?` |

Note: `cargo check` surfaced only lib errors so far; more may appear in the *bins* once
the lib compiles (the api/*.rs bins reference `db::connect`, `env`, `json_response`, etc.).
Run `cargo check --all-targets` and `cargo test` after the fixes.

## Design review verdict (full detail in the review message; key points)

1. **Stringly-typed domain** (`anti-stringly-typed`, `type-enum-states`): `action_type`,
   `status`, `outlet_type` are `String` end-to-end (`models.rs`, `db.rs`). Allowed tokens
   duplicated as `&[&str]` in `llm.rs:28-51`. `status_for` (`db.rs:339`) has a
   `_ => "suspended"` catch-all → unknown types become suspensions silently.
2. **`violations: Value`** (`db.rs:50,74`) should be `Vec<String>` (store as `TEXT[]`
   like `platforms`, or `Json<Vec<String>>`).
3. **Derived data persisted**: `status` computed by `status_for` at write time and
   stored → drifts out of sync with `action_type`. Prefer removing the column and
   deriving on read (or a CHECK/trigger).
4. **Module cohesion** (`proj-mod-by-feature`): `db.rs` mixes persistence with domain
   coercion (`nonempty` :334, `status_for` :339, `coerce_action_date` :347 belong in the
   domain layer).
5. **Swallowed failures** (`anti-empty-catch`): `fetch_article` `.ok()?` (`news.rs:142`);
   LLM rows dropped with `.filter_map(..., .ok())` (`llm.rs:145`); `compliance_score`
   silently filtered (`scrape.rs:89`); `stats()` last_run hand-rolls 9 `.unwrap_or`s
   (`db.rs:283-298`); `grouped_counts`/`grouped_counts_list` are dupes of the same query
   (`db.rs:310-332`).
6. **Fresh `PgPool` per request** in every handler (`actions.rs:14`, `stats.rs:13`, …) and
   a fresh `reqwest::Client` per call (`news.rs:35`, `llm.rs:74`) → use process-wide
   `OnceLock` pool + shared client.
7. **Sentinel `run_id: 0`** (`scrape.rs:11-68`): report created with fake 0, patched after
   success; `error: Option<String>` field is dead (never written). Pass run id down instead.
8. **Over-engineering**: 6 binaries × identical `#[tokio::main] + run(service_fn)` boilerplate;
   generic `query_params<T>` used once; count query duplicates the WHERE filter (use
   `COUNT(*) OVER()`); `brands.rs`/`cities.rs` differ only by column name → one
   `/api/dimensions` endpoint.
9. **Data-correctness bugs**: `coerce_action_date` window `(-2..=45)` snaps older dates to
   *today* (`db.rs:347`) — drops the window; `seen_urls` filters on `published_at > cutoff`
   so NULL published rows are never treated as seen and get re-LLM'd daily.
10. **Security**: `has_secret` fails **open** when `CRON_SECRET` unset (`cron_scrape.rs:51-60`);
    `is_cron` trusts the spoofable `user-agent` header. Fail closed (e.g. require secret and
    treat UA cron as advisory-only).

What's right (keep): `UNIQUE (source_url, establishment, action_date)` + `ON CONFLICT
DO UPDATE`; `fetch_runs` audit trail; Gemini 429 backoff; boundary validation of LLM
output; CI workflow intent (fmt/clippy/test — but has never been green).

## Approved refactor plan (pending user go-ahead)

1. Make it compile (blocker list above).
2. Enums for `ActionType` / `OutletType` (`FromStr` + `Display` + serde snake_case
   renaming); derive `status` on read, delete `status_for` catch-all.
3. `violations: Vec<String>`; `coerce_action_date` → trust reported date (no snapping);
   move domain coercers out of `db.rs`.
4. Propagate errors instead of swallowing (log truncated offending LLM row at minimum).
5. `OnceLock<PgPool>` + one shared `reqwest::Client`.
6. Kill dead code: `news::batch_json`, `models::default_action_date`, unused `futures` dep.
7. Fix `run_id` plumbing; remove dead `error` field.
8. Slim duplication (dimensions endpoint, `serve(handler)` helper, `COUNT(*) OVER()`).
9. Fail-closed `CRON_SECRET` handling.

## Status / what remains

DONE: scaffold; full refactor; local build green (`cargo check --all-targets`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --lib` all pass);
git history + public GitHub repo created; Vercel deploy pending.

NOT started: Vercel deploy + env vars, verify cron fires, optional manual
seeding of historical data (e.g. the Aug 14–16 suspension list).

## Refactor notes (what changed vs the original review)

- Enums `ActionType`/`OutletType` (`FromStr` + `Display` + snake_case serde);
  `status` is now a Postgres **generated column** derived from `action_type`
  (no stored-writable status, no `status_for` catch-all); `action_type` has a
  CHECK constraint.
- `violations` is `Vec<String>` stored as `TEXT[]` (was `Value`/JSONB).
- Domain coercers (`nonempty`, `coerce_action_date`) moved from `db.rs` to
  `models.rs`; `coerce_action_date` now trusts a parseable reported date
  instead of snapping to today.
- `db::pool()` returns a process-wide `OnceLock<PgPool>` (runs migrations
  once); `lib::http_client()` shares one `reqwest::Client`. Per-request pools
  and clients are gone.
- `list_actions` uses `COUNT(*) OVER()` (no duplicated count query);
  `brands`/`cities` share `dimension_counts`; `stats` uses a typed `RunSummary`.
- `fetch_article`/LLM rows no longer swallow errors (logged + fall back);
  `seen_urls` treats NULL `published_at` rows as seen.
- `run_id` is plumbed through (no sentinel `0`); dead `error` field/`batch_json`/
  `default_action_date`/`futures` dep removed.
- `/api/cron_scrape` fails closed: non-cron calls require `CRON_SECRET`
  (see README caveats).
- `lib::serve()` helper removes the per-bin `run(service_fn(handler))` boilerplate.
- Fixed a pre-existing failing test: `strip_code_fences` now strips the language
  tag from fenced LLM output.

## Secrets needed from user (in order of use)

- `DATABASE_URL` — Neon pooled connection string ("the neon db we use").
- `GEMINI_API_KEY` — from aistudio.google.com (free).
- `CRON_SECRET` — random string (fail-closed usage after refactor).
- Vercel/GitHub are already wired: `gh` authenticated as **abhishekshree**; `vercel` CLI
  installed.

## Deploy checklist (after refactor)

1. `cargo check --all-targets` && `cargo test` green locally.
2. `git init`, conventional commit, push to new public repo `abhishekshree/fda-mumbai-tracker`.
3. Import in Vercel → env vars → deploy → verify `/api/stats`, `/api/actions`, dashboard.
4. Verify cron fires daily (check via `fetch_runs` table / Vercel cron logs).
5. Optionally seed historical data (past raids, e.g. the Aug 14–16 suspension list) manually.