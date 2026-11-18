# Architecture audit — `fda-mumbai-tracker`

**Status:** implemented refactor on branch `refactor/architecture-audit`. The audit below is retained as the design record; Section 2 now annotates each finding with its implementation outcome.

**Scope (of the original audit):** read-only review of all Rust sources (~2,700 LOC), migrations, prompts, config, CI workflows, and the TypeScript read side. The first pass was written before the LLM triage rewrite; every finding below was then re-checked against the then-current working tree and amended where necessary. Section 6 records exactly what was corrected.

**Audit-baseline note:** at audit time the triage rewrite was uncommitted and included `src/llm/triage.rs`, `src/llm/mod.rs`, `src/llm/genai.rs`, `src/rules.rs`, `src/rules.toml`, and the new `src/prompts/triage.txt`. It removed the keyword triage rules, the 25-city hint list, and the headline-derived name/city fallback, and replaced them with an LLM relevance pass that returns typed `Verdict`s. Findings affected by this are marked **Amended**; the one new finding is marked **New**. That rewrite is now part of the baseline this refactor started from.

**Audit-baseline verification (working tree at audit time, before this refactor):**

- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test` — 45 passed, 1 ignored (`live_triage_demo`, a live-network test).
- `.github/workflows/scrape.yml:54` greps stderr for `llm batch .* failed`. Verified by direct test: this pattern **does not match** the message `"llm batch failed"` emitted at `src/llm/mod.rs:220` (it requires a second ` failed` after the wildcard). The CI "Failed subtasks" counter therefore undercounts extraction failures. That grep has since been deleted (see Implementation outcome).

---

## Implementation outcome

This branch is the implemented refactor, not an audit-only deliverable. The four commits in `5f54095..HEAD`:

| Commit | What it did |
|---|---|
| `ec7c7c9` docs: record architectural audit and agreed refactor scope | Added this document — the audit, the agreed design, and the incremental plan. No code change. |
| `29f598b` refactor: add validated domain types and centralised config | Added `src/config.rs` (all env reads in `Config::from_env`), `src/domain/mod.rs` (`Article`, `Candidate`, `CoveredCity`), `src/error.rs` (`Rejection`), `src/report.rs` (`RunMetrics`, `RunReport`, `StoreOutcome`). Types only, not yet wired in. |
| `326d5b7` refactor: wire validated pipeline and drop redundant layers | The bulk of the change. Pipelines take `&Config` and return `RunReport`; deleted `src/bin/refresh.rs`, `replace_actions`, the LLM collapse pass and `src/prompts/dedupe.txt`, the salvage path, the counter tuples, the `&mut usize` salvage counter, the broken CI grep, and the prompt string-presence tests. Updated the runbook and the TS label comments. |
| `ee7f3e2` fix: align dedup windows and restore invariant tests | Widened the `same_event` pre-check to ±10 days to match the SQL `daterange` canonicalisation, added the `action_type` filter so a `reopened` row is not swallowed, and restored invariant tests (`same_event` acronym/venue cases, action-code/SQL/dashboard parity, missing-date and unknown-outlet cases). |

**Net code change:** `5f54095..HEAD` excluding this audit document is **932 insertions and 1,124 deletions across 23 files — net −192 lines**. (Including the document, the diff is larger; the document is design record, not shipped code.)

**Verification at the final landed tree:**

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test` — 44 passed.
- `cargo nextest run --profile ci` — pass.
- `cargo build --release` — succeeds.
- All three binaries (`cache_bust`, `local_scrape`, `migrate`) build.

These results cover the full tree including finding 7 and the web-status deduplication.

### Scope decisions actually taken

- No repo-wide `thiserror` migration. `Rejection` is a fieldless `strum::Display` enum (`src/error.rs`), not an error hierarchy; `anyhow` stays at I/O boundaries.
- `parse_drops` was rejected as overcomplication — no helper was added just to report dropped records.
- The canonical-key `canonical_outlet` column and one-off backfill (finding 1) were descoped by the owner; the SQL schema is unchanged.
- A real end-to-end / database run is deferred: this environment has no scratch database and no API key. No live LLM/RSS path was exercised.

### Protobuf decision: DEFER

Keep the hand-maintained vocabulary and the parity test; do not introduce generated wire types.

- The database is already the shared schema between Rust and TypeScript; there is no wire boundary between the two to justify a schema language.
- The SQL `CHECK`/`CASE`, the prompt prose, and the TS label maps are not generator-producible from a proto definition — a generator could not remove those copies.
- The parity test at `src/models.rs:148-162` already removes the duplication that is eliminable, by checking Rust codes against the SQL `CHECK` and the TS label map.

---

## 1. High-level assessment

*This section and the findings below describe the audit baseline; implementation outcomes are annotated under each finding and summarised above.*

### What the system actually is

A single-crate ETL with five library modules and four thin binaries, writing to one Postgres table that a Next.js dashboard reads directly. The crate shape is appropriate. Do not restructure it.

```
RSS (raw) → enrich (processed) → LLM triage (relevant) → LLM extraction (candidate)
          → quality gate → build_rows (insert DTO) → dedup (×5) → Postgres → Next.js
```

### What is genuinely good

- `ActionType` and `OutletType` are real single-sourced domain types: strum + serde + `Display` with `snake_case` (`src/models.rs:29-81`), with round-trip tests (`src/models.rs:161-181`) and prompt-coverage tests (`src/llm/mod.rs:285-297`, `src/llm/triage.rs:88-98`). That is the right instinct.
- Bundling quality/news rules into one TOML with a validation test (`src/rules.rs:32-39`, `src/rules.rs:45-65`) is a good design; the recent removal of the triage rules from it was also correct — those rules had become a second, weaker copy of what the LLM now does.
- Pure, unit-tested functions at the messy edges: `extract_snippet`, `same_event`, `quality::check`, `parse_llm_text`, `parse_verdicts`, `build_rows`. Tests follow Arrange-Act-Assert with failure messages.
- Trust-boundary ceilings exist and are documented (`MAX_ARTICLE_BYTES`, snippet truncation, `sanitize_action` clamps, `BATCH_SIZE`, the 600-char triage snippet at `src/llm/genai.rs:85`).
- The system degrades rather than going dark: article fetch failure falls back to the headline, triage failure passes items through, extraction failure salvages from triage verdicts, Gemini failure falls back to OpenRouter. The *quality* of that degradation is uneven — see findings 3 and 11.

### The central problem

**This is a pipeline of string-shaping stages, not a pipeline of typed state transitions.** Every stage takes loose `String`/`Option<String>`, decides ad hoc what to accept, and hands loose strings to the next stage. The same real-world concepts — what counts as "the same event", which cities exist, what `status` means, what an error is — are each defined in three to six places across Rust, TOML, SQL DDL, prompt prose, and TypeScript.

Nearly every production incident in `git log` (Sept-2026 junk, MCA/BKC duplicates, zero-data days, quota exhaustion, delisted models) was fixed by adding another gate, fallback, or dedup layer at the point of pain rather than tightening the model. The triage rewrite is the latest example: it added a whole new LLM pass, but kept the old validation, dedup, and reporting machinery around it.

Verdict: the difficulty is not Rust idiom — it is that **domain decisions have no owner**. The codebase is operationally resilient but semantically incoherent: no single place can answer "what is a valid action record, and how do we know two records are the same event?"

---

## 2. Findings

Severity is about architectural impact, not style. "Verification" states how the finding survived the re-check.

### 1. Five competing definitions of "same event" — **Critical** — *Partial (audit verdict: Confirmed)*

**Implemented status — partial.** The five policies were reduced to two. The LLM collapse pass (`collapse_dupes`, `src/prompts/dedupe.txt`) and the replace-window in-memory dedup are deleted; `same_event` in `upsert_actions` (`src/db.rs:100-148`) is now the one Rust dedup authority, with the SQL `EXCLUDE` constraint as the final backstop. `ee7f3e2` widened the pre-check from ±5 to ±10 days so it matches the SQL `daterange` canonicalisation, and filtered candidate rows by `action_type` so a `reopened` row is not swallowed. A canonical `canonical_outlet` key and the one-off backfill were descoped by the owner; no migration was added.

**Locations**

| Authority | Definition | Where |
|---|---|---|
| A. Extraction dedup | `(source_index, lower(establishment), action_type)` | `src/llm/mod.rs:166-177` |
| B. LLM collapse pass | Semantic grouping, **explicitly across action types** | `src/prompts/dedupe.txt:11`, `src/llm/genai.rs:345-364` |
| C. Insert-time heuristic | `same_event`: fuzzy name, ±5 days, **ignores action_type and city** | `src/db.rs:214-270`, used at `src/db.rs:138-177` |
| D. Replace-window dedup | `same_event` **plus city equality** plus ±5 days | `src/db.rs:196-207` |
| E. Database constraint | `lower(establishment)` + `action_type` + overlapping ±5-day daterange, **no city, no fuzz** | `migrations/0002_no_dupe_events.sql:13-18` |
| F. Manual backfill | Regex over SQL text + per-row savepoints | `docs/manual-backfill.md:119-166` |

(The new triage pass adds a sixth `HashSet<usize>` dedup at `src/llm/mod.rs:39,56,70` — but that dedups *input indices*, not events, so it is harmless and not counted above.)

**Current design.** None of these is a layer of one policy; they are five different answers to the same question. B says group an inspection and the suspension it triggered; E keys on `action_type` and therefore allows both; C forbids both. C ignores city, D requires it, E ignores it. A is keyed on an array index that has nothing to do with identity.

**Concrete contradictions already in the repository:**

- `README.md:48-49` promises "Suspension is not cancellation… tracked as a separate `reopened` record." But C compares only the name within ±5 days (`src/db.rs:155-160`, with no `action_type` predicate in the candidate query at `src/db.rs:145-154`) — a reopening inside that window is silently `continue`d away. Documented behaviour and implemented behaviour disagree.
- `docs/manual-backfill.md:72-74` warns about namesakes in different cities. D would keep them (city differs), C and E would collapse them (same name). Which one wins depends on which binary you ran.
- D can admit a pair that E rejects: two rows with identical establishment/action_type/dates but different cities pass the in-memory check at `src/db.rs:198-204`, then hit the `EXCLUDE` constraint on `INSERT` → the whole `replace_actions` transaction aborts. This is a reasoned hazard, not an observed failure.

**Root cause.** Each duplicate incident produced a new local fix. Migration 0003 is explicit: "this heals existing rows"; acronym matching was bolted onto `same_event` for the August 2026 MCA/BKC case. Nothing forced reconciliation with the existing four.

**Consequences.** Silent data loss (C skips legitimate rows), nondeterministic results across binaries, transaction aborts, and a backfill runbook that has to parse SQL with regex to find out what the database rejected.

**Proposed design.** One canonical identity, one authority:

1. Compute a `canonical_outlet` key once, in one pure function, when a `Candidate` becomes an insert row: lowercase, strip venue words (`GENERIC_WORDS` already exists at `src/db.rs:215-223`), expand known acronyms. Store it as `canonical_outlet TEXT NOT NULL`.
2. Make the database the single dedup authority: one `EXCLUDE USING gist (canonical_outlet WITH =, daterange(action_date - 5, action_date + 5, '[]') WITH &&)`.
3. Decide **one** answer to the action-type question. Given the dedupe prompt and README, they are one saga → drop `action_type` from the exclusion key. Then record all action types on the surviving row, or model `reopened` as state on that row rather than a separate row.
4. Delete `same_event`, delete D's in-memory dedup, and delete the LLM collapse pass (or make it propose canonical keys rather than returning indices to drop).

**Implementation caveat.** Acronym expansion ("MCA" → "mumbai cricket association") cannot be expressed in a SQL expression, so the backfill needs a one-off Rust pass: add a nullable `canonical_outlet`, run a small binary that recomputes it for every existing row using the shared normalization function, set `NOT NULL`, add the new constraint, drop the old one. Do not reimplement the heuristic in SQL — that would recreate the same duplication.

**Migration scope:** `src/db.rs`, `src/llm/mod.rs`, `src/llm/genai.rs`, `src/prompts/dedupe.txt`, a new migration + one-off backfill binary, `src/scrape.rs`, `docs/manual-backfill.md`. This is the largest change and must land after the data model is settled.

---

### 2. Failures are not visible in the run report — **Medium** — *Done (audit verdict: Amended and descoped)*

**Implemented status — done.** `triage_failures`, `extraction_failures` and `rejected` are counted in `RunReport.metrics` (`src/report.rs:3-9`), written into `fetch_runs.details` by `finish_run` (`src/db.rs:229-242`) and printed in the completion line; the broken CI grep is deleted. `quality::check` now returns `Result<(), Rejection>` where `Rejection` is a fieldless strum enum (`src/error.rs:8-20`) — not `&'static str`, and without adding `thiserror`. No repo-wide error migration.

**Locations.** `src/llm/quality.rs:6` returns `Result<(), &'static str>`. `src/llm/genai.rs:58,60,136,149,252,264,266,324,332` constructs errors as formatted `anyhow!` strings. The new triage code adds more (`src/llm/triage.rs:36,39`). There is no `thiserror` in `Cargo.toml`.

**Current design.** Everything is `anyhow`, and most operational failures are logged and swallowed:

- Failed RSS query → `warn!`, continue with fewer items (`src/news.rs:126-131`)
- Failed article fetch → `warn!`, substitute the headline as snippet (`src/news.rs:304-313`)
- Failed triage batch → `warn!`, pass all items through with placeholder verdicts (`src/llm/mod.rs:66-82`)
- Failed `recent_events` or dupe-collapse LLM → return rows unchanged (`src/scrape.rs:94-100`, `src/llm/mod.rs:247-253`)
- Invalid LLM record or triage verdict → `warn!`, drop (`src/llm/genai.rs:269-277`, `src/llm/triage.rs:42-48`)
- Panicked fan-out task → `error!`, discard the `JoinError` (`src/news.rs:56-60`)

**Underlying problem.** Failures are logged but not counted, so a degraded run is indistinguishable from a clean one. The *reporting* gap is the high-value part. The lack of a typed error vocabulary is real but lower value here: nothing in the crate discriminates on errors — `chat_text` retries every failure identically, `collapse_dupes` warns and moves on, and `quality::check`'s reason is only logged. `AGENTS.md` already sanctions `anyhow` at I/O boundaries, which is where these errors originate.

**The smoking gun.** Observability moved into shell regex at `.github/workflows/scrape.yml:54-56`:

```bash
failed="$(grep -c 'llm batch .* failed\|rss query failed\|article fetch failed\|enrich task join error' /tmp/scrape.log || true)"
```

As noted above, that first pattern does not even match the message the code emits. The runbook says "Green runs mean nothing: always read `actions_upserted`" (`docs/manual-backfill.md:182`). That is the actual defect: observability lives in log text instead of the run report.

**Proposed design — now (cheap).** Keep `anyhow`. Do finding 6's named-report refactor, add a few counters to `ScrapeReport` (`triage_failures`, `extraction_failures`, `rejected`), print them in the completion line, and delete the broken grep from CI. Roughly 30 lines, no new dependency. Keep salvage as a *report field* rather than a silent mutation through `&mut usize` (`src/llm/mod.rs:202-231`). The swallowed RSS/article/collapse failures are deliberate degradation under quota and IP blocks — keep them; count them.

**Proposed design — deferred (owner's decision).** `Result<(), &'static str>` in `src/llm/quality.rs:6` is acknowledged as a poor error type and is on the list, but after the higher-value items. When reached, replace it with one small local enum (a `thiserror`-derived `Rejection` or `QualityRejection` is fine), and only add further custom error types if an actual caller needs to discriminate on them — e.g. selective retry of network but never parse errors. Do **not** migrate every `Result` in the crate to a six-enum hierarchy; there is no consumer for it today.

**Migration scope:** now — `scrape.rs`, `genai.rs`, `llm/mod.rs`, `scrape.yml`. Deferred — `quality.rs` and any small local error type.

---

### 3. Raw strings cross every boundary; validation happens six times with silent defaults — **High** — *Done (audit verdict: Amended)*

**Implemented status — done.** `Article` and `Candidate` are separate types and `Candidate::try_new` (`src/domain/mod.rs:105-147`) is the single structural validation point; the flat `db::ActionInsert` DTO is gone. There is no `Utc::now()` fallback — undated records are rejected with `Rejection::NoActionDate`. Unknown outlet codes become `None`, covered cities are parsed by `CoveredCity::parse`, and out-of-range scores are filtered to `None`. The extraction/salvage split is gone, so `quality::check` applies to every candidate (`src/llm/mod.rs:142-149`).

**Locations.** `LlmAction` (`src/models.rs:83-113`) has 14 fields, almost all `String`/`Option<String>`/`Vec<String>`, with no invariants. The same values are processed by:

1. serde aliases + `deserialize_string_vec` — `src/models.rs:95-120`
2. `parse_verdicts`, dropping invalid triage verdicts — `src/llm/triage.rs:33-50`
3. `parse_llm_text`, dropping invalid records — `src/llm/genai.rs:249-280`
4. `quality::check` — `src/llm/quality.rs:6-37`
5. `sanitize_action` clamps and truncates — `src/llm/genai.rs:282-305`
6. `nonempty` / `coerce_action_date` / `OutletType::from_code` / compliance-score filter in `build_rows` — `src/scrape.rs:135-166`, `src/models.rs:141-154`

Then `ActionInsert` (`src/db.rs:65-84`) stores the same loose strings again.

**Silent substitutions that destroy information:**

- `coerce_action_date` falls back to `Utc::now()` when neither a reported date nor a publication date exists (`src/models.rs:147-154`). This fabricates a domain fact on a public, source-linked tracker.
- `OutletType::from_code` maps every unknown string to `Other` (`src/models.rs:76-81`), while an unknown `ActionType` causes the whole record to be dropped by serde (`src/llm/genai.rs:269-277`). Two enum-like fields, opposite error policies.
- Out-of-range `compliance_score` becomes `None` (`src/scrape.rs:155`) — you cannot tell "not reported" from "reported nonsense".
- Out-of-scope `city` values from the triage LLM are checked again by a *different* mechanism in `build_rows`: a case-insensitive `strip_prefix` against `IN_SCOPE_CITIES` (`src/scrape.rs:121-143`). The prompt uses a five-item English list (`src/prompts/triage.txt:23-24,36`); Rust uses prefix matching. Validation is duplicated in two languages with two semantics.
- **The quality gate is still applied only to LLM extraction output, never to the salvage path** (`src/llm/mod.rs:145-159`). The comment justifies this by saying salvage rows "come from triage verdicts, not from this model's free-form generation" — but triage is also an LLM, and its `establishment`/`city` are equally unvalidated. A verdict naming `"FDA"` with city `"Mumbai"` would be rejected by `quality::check` as a regulator (`src/llm/quality.rs:24-26`) but survives on the salvage path.

**Root cause.** No boundary type exists, so every function that touches a string feels obliged to defend itself. Validation is a distributed habit rather than a stage. The triage rewrite added a seventh place to validate without removing any of the old ones.

**Proposed design.** Three states with `TryFrom` between them, no framework:

- `RawItem` — RSS output (current `NewsItem`).
- `Article` — after enrichment: canonical `final_url`, extracted snippet, publication date. Produced by `news::enrich`.
- `Candidate` — after extraction and validation: non-empty establishment, covered city, `ActionType`, a real `NaiveDate`, bounded lists. `TryFrom<(ExtractedAction, Article)> for Candidate` owns `Rejection` and replaces `quality::check` + `sanitize_action` + `build_rows` validation.
- `ActionInsert` stays as the flat DB DTO, built from `Candidate` in one infallible step.

Specific corrections: reject undated records instead of calling `Utc::now()`; map unrecognised outlet codes to `None` (the DB column and TS type are already nullable) rather than the `Other` error sink; apply the same `Candidate` validation to extraction and salvage alike.

**Migration scope:** `models.rs`, `news.rs`, `scrape.rs`, `llm/{mod,genai,quality,triage}.rs`. Net code should go **down** — six validation stages collapse into one.

---

### 4. Three write paths with divergent semantics — **High** — *Partial (audit verdict: Confirmed)*

**Implemented status — partial.** `replace_actions` and `src/bin/refresh.rs` are deleted; `db::upsert_actions` is the only Rust write path. The Python repair script in `docs/manual-backfill.md` remains by deliberate choice — no `--since` repair mode was added to the Rust binary, so two repair mechanisms (one Rust, one Python) still exist.

**Locations.**

- `db::upsert_actions` (`src/db.rs:138-177`) — daily path from `src/scrape.rs:73`. Fuzzy pre-check skips; `ON CONFLICT (source_url, establishment, action_date) DO UPDATE` updates only violations/platforms/details.
- `db::replace_actions` (`src/db.rs:183-212`) — used only by `src/bin/refresh.rs:30`, which no workflow invokes (`Cargo.toml` has no `[[bin]]` for it; Cargo auto-discovers `src/bin/*.rs`). `DELETE WHERE published_at >= $1 OR action_date >= $1::date`, then in-memory dedup, then `DO NOTHING`.
- `docs/manual-backfill.md:80-166` — a Python script that parses SQL statements with `str.split("ON CONFLICT...")` and catches `ExclusionViolation` with a regex.

**Underlying problem.** "Refresh a window" and "repair a gap" were treated as new write mechanisms rather than parameters of one mechanism. The Python path exists because the Rust path does not expose the needed operation.

**Consequences.** Identical input through different binaries yields different database state. The broad `OR` in the DELETE can remove rows whose `published_at` is recent but whose `action_date` is old, and leaves NULL-`published_at` rows untouched. The city-vs-constraint mismatch in finding 1 can abort the transaction.

**Proposed design — the lazy version.** Delete `src/bin/refresh.rs` (auto-discovered, so no `Cargo.toml` edit needed) and `replace_actions` if nothing automated uses them. Keep exactly one Rust write path and one repair path. Make the repair path call the same Rust binary with a `--since` argument that drives a proper `upsert_actions` window; delete the Python from the runbook. If a true replace is required, express it as one `WriteMode` parameter on `store_actions` that shares the same insert primitive and the same dedup key.

**Migration scope:** `src/db.rs`, `src/bin/refresh.rs`, `docs/manual-backfill.md`.

---

### 5. Domain vocabulary duplicated across five artifacts — **High** — *Partial (audit verdict: Amended)*

**Implemented status — partial.** `CoveredCity` is now an enum used by `Candidate::try_new` (`src/domain/mod.rs:12-46`), replacing the string-prefix city check, and `action_codes_match_sql_and_dashboard` (`src/models.rs:148-162`) checks Rust codes against the SQL `CHECK` and TS label map. Still open: no Rust `Status` enum or SQL `CASE` parity test, the SQL `ELSE 'suspended'` catch-all remains, and the prompts are still unstructured prose. The TS changes are comments pointing at the SQL/Rust sources, not generated labels.

**Locations.**

- *Action/outlet codes:* Rust enum (`src/models.rs:45-74`), `CHECK` constraint (`migrations/0001_init.sql:10-13`), extraction prompt (`src/prompts/system.txt:27-28`), triage prompt (`src/prompts/triage.txt:34-35`), TS label maps (`web/src/lib/format.ts:1-24`), TOML quality rules (`src/rules.toml:4-18`).
- *Covered cities:* `IN_SCOPE_CITIES` — 5 entries (`src/scrape.rs:121`), extraction prompt (`src/prompts/system.txt:26`), triage prompt — the same 5 names listed **twice** in one file (`src/prompts/triage.txt:23-24` and `:36`), plus a hardcoded list in `src/scrape.rs:240-267`. The old 25-city list in `rules.toml` is gone; that inconsistency no longer exists.
- *Status:* generated SQL column with a `CASE` mapping 7 action types to 3 statuses (`migrations/0001_init.sql:15-23`), TS `STATUS_CONFIG` with the same 3 (`web/src/lib/format.ts:26-51`), `Filters.status` typed as bare `string` (`web/src/lib/types.ts:12`).

**Underlying problem.** There is no shared vocabulary owner. Tests exist for some of this (`src/models.rs:161-181`, `src/llm/mod.rs:285-297`, `src/llm/triage.rs:88-98`, `src/rules.rs:45-65`), but they are *string-presence* tests over prompt text, not semantic parity tests, and they do not cover the SQL `CHECK`/`CASE` or the TS maps. **Specifically, the test at `src/llm/triage.rs:88-98` checks that the triage prompt names every action code but not that it names the covered cities — even though triage is now the gate that decides city.** The city check remains only on `SYSTEM_PROMPT` (`src/llm/mod.rs:294-296`).

**Consequences.** Adding a city now requires coordinated edits in three places, two of which are unstructured prompt text; a miss silently loses records. Adding an action type requires updating the enum, the SQL `CHECK`, the SQL `CASE`, two prompts, the TOML quality rules, the TS maps, and two prompt tests. The compiler helps with none of the non-Rust edits.

**Proposed design.** Keep the existing tests but make the vocabulary explicit:

- One `CoveredCity` enum in Rust (the 5 real cities), used by `build_rows` and by a test that *both* prompts mention all variants.
- Keep `status` as a SQL generated column (a good design for a directly-read table), but add a `Status` enum in Rust with `From<ActionType>` and a test asserting the Rust mapping equals the SQL `CASE`. TS labels can stay a map; add a parity test rather than a codegen pipeline for 3 values.
- The SQL `CASE` has an `ELSE 'suspended'` catch-all (`migrations/0001_init.sql:21-22`). With a closed action-type enum and a `CHECK` constraint, that is defense against a state the schema already prevents. Make the mapping exhaustive in one place.

**Migration scope:** `models.rs`, `scrape.rs`, both prompt files, a new migration if the `CASE` changes, `web/src/lib/format.ts`.

---

### 6. Observability counters threaded as tuple elements through nine signatures — **High** — *Done (audit verdict: Amended)*

**Implemented status — done.** `RunMetrics`, `RunReport` and `StoreOutcome` (`src/report.rs`) replace the nine counter tuples; one `&mut RunMetrics` is created in `scrape_once` and passed to the pipeline (`src/scrape.rs:60-69`) instead of a hidden parameter per helper. `finish_run` writes all five counters to `fetch_runs.details`, and the CI summary reads `triage_failures`/`extraction_failures` from the completion line instead of grepping stderr.

**Locations.**

| Function | Signature |
|---|---|
| `chat_text` | `-> (Result<String>, usize)` — `src/llm/genai.rs:39-44` |
| `triage_batch` | `-> (Result<Vec<Verdict>>, usize)` — `src/llm/genai.rs:72-76` |
| `extract_batch` | `-> (Result<Vec<LlmAction>>, usize)` — `src/llm/genai.rs:99-106` |
| `collapse_once` | `-> (Result<Vec<usize>>, usize)` — `src/llm/genai.rs:179-184` |
| `triage` | `-> Result<(Vec<Verdict>, usize)>` — `src/llm/mod.rs:28-32` |
| `collapse_dupes` | `-> (Vec<usize>, usize)` — `src/llm/mod.rs:236-241` |
| `extract` | `-> Result<(Vec<LlmAction>, usize)>` — `src/llm/mod.rs:88-92` |
| `run_with_window` / `run_scrape` | `-> Result<(i64, ScrapeReport)>` — `src/scrape.rs:19-31` |
| `replace_actions` | `-> Result<(usize, usize)>` — `src/db.rs:183-187` |

Plus `salvaged(..., failed: &mut usize)` (`src/llm/mod.rs:202-208`) and a duplicate home for the count in `ScrapeReport.llm_calls` (`src/scrape.rs:16`). The triage rewrite added two of these signatures rather than removing any.

**Underlying problem.** A metrics concern is smuggled through every layer as an out-of-band return value. Each layer must remember to add, thread, and re-wrap it. `usize` has no meaning at any of these boundaries.

**Consequences.** Nine signatures say nothing about their real contract; adding a second counter means editing all nine; caller-side arithmetic (`src/llm/mod.rs:130,143`, `src/scrape.rs:69-72`) is where call-count bugs will hide.

**Proposed design.** Return one named `RunMetrics`/`RunReport` value from the operation that owns the count (`genai` returns `LlmOutcome { actions, calls, failures }`; `db` returns `StoreOutcome { inserted, skipped }`). `ScrapeReport` becomes the single flattened report. For a counter that must accumulate across a loop, pass `&mut RunMetrics` once at the top rather than as a hidden parameter to one helper. No trait, no framework.

**Migration scope:** `genai.rs`, `llm/mod.rs`, `scrape.rs`, `db.rs`, all binaries. Low risk, high readability payoff — do it early.

---

### 7. Provenance is a positional index into two parallel vector spaces — **Medium** — *Done (audit verdict: Amended)*

**Implemented status — done.** Verdict indices are resolved to `&Article` at the wire boundary and the pipeline carries references from then on. `Job<'a>` is now `{ item: &'a Article }` with no `orig`; `triage()` returns `Vec<&Article>` and dedups by URL; `verdict_index` and the `base = batch_no * BATCH_SIZE` offset math are gone; `extract_batch`/`absolute` return `Vec<(&Article, LlmAction)>`; and both `items.get(index)?` lookups in `extract()` are deleted. `LlmAction.source_index` and `Verdict.source_index` remain only as required wire fields for parsing LLM JSON, consumed once at the boundary. The sole remaining guarded lookup is `chunk.get(source_index)` in `absolute`, where an out-of-range model index is warned and dropped.

**Original evidence (pre-refactor).** `LlmAction.source_index: usize` (`src/models.rs:112`) and `Verdict.source_index: usize` (`src/llm/triage.rs:23`) travelled through two remaps: a triage `base` offset (`src/llm/mod.rs:41-63`), a `HashMap<usize, Verdict>` plus `items.get(index)?` and sort (`src/llm/mod.rs:93-133`), extraction's `remap` to `job.orig` (`src/llm/genai.rs:238-247`), and `build_rows`' `items.get(a.source_index)?` (`src/scrape.rs:139`). Four `get()?` sites could quietly discard evidence; a reorder or batch-boundary bug could attribute a record to the wrong article despite the README's source-link contract.

**Resolution.** No newtype. Each LLM stage now pairs its output with the borrowed `Article`; identity is the URL already deduplicated by `fetch_items`. Two pure tests cover multi-chunk local-index resolution and out-of-range dropping.

**Migration scope was:** `models.rs`, `llm/{mod,genai,triage}.rs`, `scrape.rs`.

---

### 8. Hidden globals and environment reads inside library functions — **Medium** — *Partial (audit verdict: Confirmed; by deliberate choice)*

**Implemented status — partial, by deliberate choice.** All environment reads moved to `Config::from_env()` (`src/config.rs:23-56`) and `&Config` is passed down from the binaries; the `db::pool()` check-then-init race is removed with a `Mutex` (`src/db.rs:19-35`); one `genai::Client` per stage now receives its keys from config. Kept deliberately: the `OnceLock`s for compile-time constants (selectors, bundled rules, shared HTTP client) and the `.expect()` on client build (`src/lib.rs:29-36`). No fake-client seam was added.

**Locations.** `OnceLock<Client>` (`src/lib.rs:14`), `OnceLock<PgPool>` (`src/db.rs:16`), `OnceLock<Rules>` (`src/rules.rs:32`), four `OnceLock<Selector>` (`src/news.rs:175-178`). `db::pool()` reads `DATABASE_URL` itself (`src/db.rs:22`); `llm::extract` reads three OpenRouter variables mid-pipeline (`src/llm/mod.rs:111-127`).

**Problems:**

- Library functions are not testable with a fake client or pool. `AGENTS.md` says "no network or DB in unit tests", which is fine, but there is no seam for a boundary test either.
- `db::pool()` does a check-then-`get_or_init` (`src/db.rs:19-30`). Two concurrent first callers can each build a pool; one is discarded. Harmless but pointless.
- `http_client()` uses `.expect()` (`src/lib.rs:32`) and release uses `panic = "abort"` (`Cargo.toml:47`) — a client-build failure aborts the process with no report.
- The new triage pass builds a *second* `genai::Client` (`src/llm/mod.rs:36`) with no OpenRouter key, then `extract()` builds another (`src/llm/mod.rs:129`). Wasteful, and a symptom of config not being resolved once.
- The `// ponytail:` comments (`src/llm/mod.rs:16,110,242`) mark deliberate shortcuts. Some are fine; the env reads and per-stage client builds are now load-bearing.

**Proposed design.** Keep the `OnceLock`s for compile-time constants (selectors, bundled rules) — those are genuinely free. Move all environment reads into one `Config::from_env()` called by each binary, and pass `&Config`, `&Client`, `&PgPool` down. Replace the double-checked pool init with one construction in `main`. Replace `.expect()` on the client with a propagated error.

**Migration scope:** `lib.rs`, `db.rs`, `llm/mod.rs`, `news.rs`, all four binaries.

---

### 9. Enrichment mutates `NewsItem`; identity changes when fetch fails — **Medium** — *Partial (audit verdict: Confirmed; by deliberate choice)*

**Implemented status — partial, by deliberate choice.** `Article` is a separate type whose `url` is set once by `news::enrich` and never mutated (`src/domain/mod.rs:48-69`), and `Candidate.source_url` is copied from it, so downstream code cannot see the mid-pipeline identity change. Not changed: `fetch_items` still dedups on the Google News URL, and there is no one-off cleanup of mixed Google/publisher URLs already in the table. Full resolution was descoped with the canonical-key work (finding 1).

**Locations.** `news::enrich` (`src/news.rs:264-323`), `fetch_items` dedup (`src/news.rs:133-134`), `db::seen_urls` (`src/db.rs:54-63`).

**Current design.** `NewsItem.url` starts as a Google News redirect URL. On successful fetch it is overwritten with `final_url` (`src/news.rs:301`); on failure it stays the Google URL and the snippet is set to the title (`src/news.rs:310-312`). `fetch_items` dedups on the Google URL; `enrich` dedups on the final URL; `seen_urls` reads back a column that is therefore a **mix** of Google URLs and publisher URLs.

**Underlying problem.** One type represents both raw and processed data, and its identity field changes meaning depending on whether a network call succeeded.

**Consequences.** The same article fetched once successfully and once unsuccessfully is stored under two different `source_url` values, so `seen_urls` cannot recognise the second one. The exact-constraint key includes `source_url` (`migrations/0001_init.sql:35`), so dedup depends on network luck; the fuzzy layer masks most of this, which is why it has gone unnoticed. The triage prompt now sees the mutated snippet (`src/llm/genai.rs:85`), so a failed fetch changes the triage input too.

**Proposed design.** Resolved for free by finding 3: make `Article` a separate type with `source_url: String` set once at the enrichment boundary, and keep `original_url` if provenance needs both. Dedup on one identity notion — the canonical publisher URL, with the Google URL as a fallback only when no publisher URL exists.

**Migration scope:** `src/news.rs`, `src/db.rs`, `src/scrape.rs`; possibly a one-off data cleanup for mixed URL forms.

---

### 10. Tests lock string details, not the risky seams — **Medium** — *Partial (audit verdict: Amended)*

**Implemented status — partial.** The test-hygiene pass removed the string-presence prompt tripwires and the live-demo test. New invariant/seam tests cover `verdict_index` offsets across two batches, `absolute()` chunk-local translation and out-of-range drops, `CoveredCity::parse`, `Candidate::try_new` (provenance, out-of-scope city, missing date, unknown outlet, score bounds), the `quality::check` rejection table, and `same_event` acronym/venue cases. Remaining gaps: no parity test between `same_event` and the SQL exclusion constraint, and no real end-to-end run.

**Current design.** Good table-driven tests for `quality::check`, `parse_verdicts`, `parse_llm_text`, `same_event`, compliance-score bounds, and `drops_from_groups`. But `prompt_names_every_code_and_covered_city` (`src/llm/mod.rs:285-297`) and `prompt_names_every_action_code` (`src/llm/triage.rs:88-98`) assert that prompt text *contains substrings* — tripwires, not behavior tests. `live_triage_demo` (`src/llm/mod.rs:299-323`) is correctly `#[ignore]`d.

**Uncovered high-risk seams:**

- ~~`remap` / triage base-offset translation (finding 7)~~ — resolved: the offset math and positional indices are gone, so this seam no longer exists.
- parity between `same_event`, the in-memory dedup, and the SQL exclusion constraint (finding 1).
- `coerce_action_date` with neither input — the fabricated-today case is untested; existing tests only cover reported and published dates (`src/models.rs:183-195`).
- unknown outlet code → `Other` is asserted as *desired* (`src/scrape.rs:214-228`), cementing the silent-substitution policy.
- triage prompt city coverage (finding 5).
- the quality gate on the salvage path (finding 3).

**Proposed design.** Do not add a testing framework. For each refactor step, add one focused test that exercises the seam named above. Keep `test-case` tables; replace the string-presence tests with parity tests once the vocabulary has an owner (finding 5). Leave the live demo ignored.

---

### 11. The new triage pass computes typed facts and discards them — **High** — *Done (audit verdict: New)*

**Implemented status — done.** Triage is now a pure relevance filter: `Verdict` carries only `source_index` and `reason` (`src/llm/triage.rs:19-27`), and the prompt and JSON schema emit only those two fields. The salvage path is deleted. An all-batches-failed extraction now returns `Err(anyhow!("all llm batches failed"))` (`src/llm/mod.rs:164-166`) and partial batches increment `extraction_failures`, so the green zero-data run described below can no longer be reported as success.

**Locations.** `src/prompts/triage.txt:31-40`, `src/llm/triage.rs:20-31`, `src/llm/mod.rs:93-103`, `src/llm/mod.rs:202-231`.

**Current design.** The triage LLM is asked for `actionType`, `city`, `establishment`, and `reason` for every qualifying item (`src/prompts/triage.txt:31-40`). On the success path, `extract()` converts verdicts to a `HashMap` and uses **only its keys** to build jobs (`src/llm/mod.rs:93-103`); the extraction LLM then independently re-derives action, city, and establishment from the full article. Triage's answers survive only in a `debug!` log of `reason` (`src/llm/mod.rs:57-61`) and in the salvage path (`src/llm/mod.rs:209-216`). The extraction prompt already re-checks relevance and covered cities (`src/prompts/system.txt:15-18,26,36-43`), so the work is genuinely duplicated.

**Underlying problem.** Two LLM passes are computing the same domain facts from overlapping evidence, and the pipeline uses the second one while throwing away the first. That is two sources of truth for `action_type` and `city`, with no reconciliation and no test for disagreement.

**Consequences.** Extra latency and cost (5 extra calls for a 100-item run), and — more importantly — a weaker failure mode:

- If a triage batch fails, placeholder verdicts with `ActionType::Inspection`, no city and no establishment are created for **every** item in the chunk (`src/llm/mod.rs:66-82`).
- If the extraction batch for those items also fails, `salvaged` builds `LlmAction`s from those placeholders with an empty establishment and no city (`src/llm/triage.rs:55-64`).
- `build_rows` then drops them via `nonempty(Some(a.establishment.as_str()))?` and `in_scope(city)` (`src/scrape.rs:140-144`).
- But `salvaged` sees a non-empty `kept` vector, so `failed` is **not** incremented (`src/llm/mod.rs:217-230`), `failed == batch_count` is false, and the run reports success with zero inserted rows.

That is exactly the "green run, zero data" failure the runbook warns about (`docs/manual-backfill.md:11-21,182-184`). The keyword fallback it replaced could at least derive a name and city from the headline (`git show HEAD:src/llm/triage.rs`), so the rewrite reduced resilience on the double-failure path.

**Proposed design — the lazy version.** Make triage genuinely a relevance filter: have it return only the set of relevant indices plus a short `reason` for logging, and drop `action_type`, `city`, and `establishment` from `Verdict`. Then either:

- (a) treat an extraction failure as an `LlmError` in the run report instead of salvaging from data that no longer exists — consistent with findings 2 and 3; or
- (b) if salvage is required, pass the full triage verdict into `extract_batch` so the extraction model is *given* the already-derived facts instead of re-deriving them, and validate both with the same `Candidate` conversion.

Option (a) is smaller and makes failure explicit. The trade-off is losing partial records on extraction failure — but today those "partial" records are dropped by `build_rows` anyway, so nothing is actually lost, only the misleading green status.

**Migration scope:** `src/prompts/triage.txt`, `src/llm/triage.rs`, `src/llm/mod.rs`, `src/llm/genai.rs`, the CI sanity check, `docs/manual-backfill.md`.

---

## 3. Common root causes

1. **Growth by accretion.** The history is a chain of incident fixes (`fix: stop silent zero-data days`, `fix: match acronym variants`, `fix: gate extraction behind strict quality checks`). Each added a stage at the point of pain. Nothing forced reconciliation with the existing stages; the triage rewrite added a seventh validation site and two new counter tuples instead of removing any.
2. **`anyhow` adopted as the default error type.** Once every failure is a String, the only place left to put semantics is the log message — hence grep-based CI checks and a runbook that says green runs mean nothing.
3. **Type-driven design applied to two enums and stopped.** Partial typing creates the illusion of a typed domain while city, date, URL, score, status, source identity, and validity all remain primitives. It also produced opposite error policies for `ActionType` vs `OutletType`.
4. **Two consumers, no shared contract.** Rust writes and TypeScript reads the same table, so domain rules live in SQL DDL, Rust, two prompts, quality keyword lists, and TS maps. Any vocabulary change is a multi-file edit with no compiler help.
5. **Shortcuts promoted to load-bearing.** The `// ponytail:` comments mark genuinely sensible shortcuts. The problem is not the shortcuts — it is that global mutable state, mid-pipeline env reads, per-stage client construction, and silent fallbacks have become interfaces other code depends on. Promote those to real boundaries; leave the rest alone.

---

## 4. Proposed target architecture and conventions

Keep the crate layout. Change the **data flow** and the **ownership of decisions**.

```
binaries
  └─ Config::from_env()? ──► &Config, &PgPool, &Client   (explicit dependencies)
       └─ run_scrape(&Deps, &Config) -> Result<RunReport> (one typed result)
            ├─ news:     RawItem        -> Article          (canonical URL, snippet)
            ├─ triage:   Article        -> relevant indices (+ reason, no typed facts)
            ├─ genai:    Article        -> ExtractedAction   (with source attached)
            ├─ domain:   (ExtractedAction, Article) -> Candidate  [TryFrom, Rejection]
            ├─ dedup:    Candidate      -> canonical key     (one authority = DB)
            └─ db:       Candidate      -> insert            (one write path)
```

**Conventions to adopt**

- **Domain types:** an enum or newtype only where it carries an invariant *and* removes a later check. `CoveredCity` qualifies (it deletes prefix matching and prompt-drift risk). A `Score` newtype does not — a range check at `Candidate` construction is enough.
- **Data states:** `RawItem → Article → Candidate → ActionInsert`, with `TryFrom` at each arrow. No stage re-validates an earlier stage's output.
- **Validation:** one `TryFrom` owns all business rules, applied to extraction and salvage alike. Expected rejections are `Rejection` values in the report, not logged strings. No fabricated dates, no silent defaults for missing domain facts.
- **Errors:** keep `anyhow` at I/O boundaries and entry points; preserve context with `.context()`. Put failures in the run report rather than only in stderr. Define a small local error enum (via `thiserror` if useful) only where a caller actually discriminates on it; replace `Result<(), &'static str>` in `quality.rs` when that file is revisited. No repo-wide error-type migration.
- **Config:** one `Config` struct built once at the boundary, validated there, passed by reference. No `std::env::var` below `bin/`. Missing `OPENROUTER_API_KEY` is an explicit `Option<OpenRouterConfig>` so "fallback disabled" is a reportable state.
- **Side effects:** `news` and `genai` own I/O and return data; domain logic is pure; `db` owns persistence. `OnceLock` only for compile-time constants.
- **Public API:** library exposes `run_scrape`, candidate construction, and store. Helpers like `same_event`, `clamp`, `drops_from_groups`, `remap`, `fan_out` become private or disappear.
- **Module communication:** named structs instead of tuples. No function returns an unlabelled `usize` the caller must interpret.
- **Persistence:** one write function, one dedup authority. SQL constraints are the final backstop, not a second opinion.

**What I explicitly would not do:** no framework, no DI container, no validation library, no async trait layer, no codegen for three TS labels, no newtype for every field, no rewriting the LLM prompts' business semantics, no changing what counts as an enforceable action, no repo-wide `thiserror` migration.

**Standing pattern for every wave (adopted; applied in `ee7f3e2`):** after each wave, run a test-hygiene pass over old and new tests together. Delete tests that only lock a trivial pass-through, a prompt string that is not a domain invariant, a rule the type system or SQL schema already enforces, or a helper that exists only to be tested. Keep tests that lock domain invariants or cover high-risk seams. Coverage percentage is not a goal.

---

## 5. Prioritized incremental plan — landed status

Steps were shipped with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` and `cargo nextest run --profile ci` (see Implementation outcome). "Done" means landed at `ee7f3e2`, not that the original finding was fully eliminated.

| Order | Change | Status | Notes / affects |
|---|---|---|---|
| **P0-1** | `config.rs`: `Config::from_env()`, all env reads moved there, pass `&Config`. Build one `genai::Client`. Fix the `db::pool()` race and the `.expect()` on client build. | **Done** | `lib.rs`, `db.rs`, `llm/mod.rs`, 4 binaries, `.env.example`. Env reads centralised and the pool race fixed; the `.expect()`, the `OnceLock`s for constants and per-stage client construction were kept by choice (finding 8). |
| **P0-2** | Replace counter tuples with `RunMetrics`/`RunReport` / `StoreOutcome`; add `triage_failures`, `extraction_failures`, `rejected` to the run report and its printed line; delete `salvaged`'s `&mut usize` and the broken CI grep. | **Done** | `genai.rs`, `llm/mod.rs`, `scrape.rs`, `db.rs`, `report.rs`, `scrape.yml`. |
| **P1-3** | Data states: `Article`, `Candidate`, single `TryFrom` validation. Remove `Utc::now()` date fabrication, unify outlet-code handling, quality gate on both paths, city validation in one place. | **Done** | `models.rs`, `news.rs`, `scrape.rs`, `domain/mod.rs`, `llm/*`. The salvage path was deleted rather than gated, so "both paths" is now one path. |
| **P1-4** | Delete triage's duplicate typed facts; make failure explicit (finding 11). | **Done** | `triage.txt`, `triage.rs`, `llm/mod.rs`, `genai.rs`, `scrape.yml`. |
| **P1-5** | Replace both index remaps with sourced pairs; keep the `NewsItem` attached through extraction. Add the two-batch test. | **Done** | `llm/{mod,genai,triage}.rs`. Verdict indices resolve to `&Article` at the wire boundary; no `base` offset, no `orig`, no `items.get(index)?`. Two pure tests cover multi-chunk resolution and out-of-range dropping. |
| **P1-6** | `CoveredCity` enum + `Status`/`From<ActionType>` parity test; cover both prompts in the prompt test. Drop the SQL `ELSE 'suspended'` catch-all. | **Partial** | `CoveredCity` and the action-code/SQL/TS parity test landed (`models.rs`). No Rust `Status` enum, no `CASE` parity test, no migration; prompt string tests were removed rather than extended. |
| **P2-7** | One dedup authority: `canonical_outlet` column, one exclusion constraint, one-off Rust backfill; delete `same_event`, the in-memory dedup, and the LLM collapse pass. | **Partial** | `db.rs`, `llm/*`, `dedupe.txt`. The collapse pass is gone and `same_event` is the sole Rust authority with the ±10 window aligned to SQL; the canonical key and backfill were descoped by the owner. |
| **P2-8** | One write path: delete `replace_actions` and `src/bin/refresh.rs`; make the backfill runbook call the Rust binary with `--since`. | **Partial** | `db.rs`, `bin/refresh.rs`, `docs/manual-backfill.md`. Two Rust paths reduced to one; the Python runbook repair path remains. |
| **Deferred** | Replace `Result<(), &'static str>` in `quality.rs` with one small local error enum (`thiserror` is fine). Add other custom error types only if a caller needs to discriminate on them. | **Done in limited form** | `error.rs`, `quality.rs`. `Rejection` is a fieldless strum enum; no `thiserror`, no repo-wide migration. |
| **Continuous** | Replace string-presence tests with seam tests: multi-batch remap, dedup parity, config validation, undated-record rejection, fallback quality gate, triage prompt cities. | **Partial** | Test modules. The hygiene pass landed the seam tests listed in finding 10; dedup parity and the remaining seams are open. |
| **Open** | Real end-to-end / database run, including dedup behaviour against a copy of `actions`. | **Deferred** | No scratch database or API key in this environment; see Section 7. |

**Dependency logic:** P0-1 and P0-2 landed independently, as planned. P1-3 was the hinge: the dedup and triage work built on a validated `Candidate`, and P1-5 then removed the positional indices that crossed those stages. P2-7 and P2-8 stopped short of the canonical key and the runbook repair mode by owner decision; the surviving write path is already the only Rust one. The error-type question was settled with a local `Rejection` enum rather than a hierarchy. No code step remains in flight.

**Behavior to preserve explicitly:** the article-fetch, triage, and OpenRouter fallbacks; the ±5-day event window; the five covered cities; the quality thresholds in `rules.toml`; and the three dashboard statuses. The only intended behavior changes are: no fabricated dates, no silent skip of legitimate `reopened` rows, identical results across binaries, and an explicit failure signal instead of a green zero-data run.

---

## 6. Verification log — corrections to the first-pass audit

| Original claim | Re-check result |
|---|---|
| Keyword triage rules and a 25-city list in `rules.toml` conflict with the 5-city scope list | **Removed.** The triage rewrite deleted the `[triage]` TOML section, the `Signal`/`Triage` types, and the headline-derived name/city fallback. The city duplication is now between `IN_SCOPE_CITIES`, `system.txt`, `triage.txt` (twice), and a test list. |
| Quality gate skipped for the "rule fallback" | **Amended.** Still skipped, but the fallback is now built from triage verdicts (`src/llm/mod.rs:145-159`, `src/llm/triage.rs:55-64`), not keyword rules. The underlying issue is unchanged. |
| Six counter-returning signatures | **Amended to nine.** `triage` and `triage_batch` add two; `run_with_window` was already counted twice in prose. |
| One positional index remap | **Amended to two.** Triage now adds a chunk-local → absolute conversion (`src/llm/mod.rs:42,55`) before extraction's chunk-local → `orig` remap. |
| CI grep encodes the error taxonomy | **Confirmed and strengthened.** Direct test showed `grep -c 'llm batch .* failed'` returns 0 for the actual `"llm batch failed"` message, so CI undercounts failures. |
| `refresh.rs` needs a `Cargo.toml` edit to remove | **Corrected.** Cargo auto-discovers `src/bin/*.rs`; there is no `[[bin]]` entry for `refresh`. Deleting the file is sufficient. |
| Baseline build status unknown | **Verified.** `cargo clippy --all-targets -- -D warnings` clean; `cargo test` 45 passed, 1 ignored. |
| No finding about the new triage pass | **Added as finding 11.** Verdict fields are discarded on success; the placeholder verdict chain produces a green zero-data run on double failure. |
| Repo-wide `thiserror` hierarchy as P0-1 | **Dropped and descoped per owner decision.** Finding 2 is now Medium, split into a cheap "report the failures" step now and one local error enum (possibly `thiserror`-derived) later. The `&'static str` in `quality.rs` is still considered bad, just low priority. Since then, `quality.rs` returns the fieldless strum `Rejection` enum and no `thiserror` dependency was added. |

---

## 7. What was not checked, and the main risk

At audit time I had not run `cargo nextest run --profile ci`; that profile now passes for the landed refactor (see Implementation outcome). Still not done: any live LLM/RSS path, a real end-to-end / database run (no scratch database or API key in this environment), and measuring how often the double-failure path in finding 11 occurred in production logs. All dedup-abort and concurrency claims are reasoned from the code, not observed.

Main risk: **finding 1 remains partial** — two dedup authorities still exist (`same_event` in `src/db.rs` and the SQL `EXCLUDE` constraint), and the ±10 alignment is a code-level fix, not a tested parity guarantee. Any future canonical-key migration needs a dry run against a copy of `actions` before it touches production data. Finding 7 is landed and verified in the final tree; no code work remains in flight.
