# AGENTS.md

## Rust code review

Any review of Rust code in this repo must load the `idiomatic-rust` skill
(global, ID `idiomatic-rust`) and complete its coverage contract: all 15
patterns and all 23 checklist items, each with a verdict. Sampling a few
patterns is not a review.

- Run `cargo clippy --all-targets -- -D warnings` and `cargo test` first — CI
  fails on any warning.
- Cite `file:line` for every finding.

## Rust style

Applies to all Rust in this repo. Before committing: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`.

### Errors

- Fallible work returns `Result`; propagate with `?`. `anyhow::Result` at
  entry points and I/O boundaries, `.context()` on the external call.
- No `unwrap()` in production paths. `expect()` only where the invariant is
  visible in the same function (static selector, shared client build).
- No panicking indexing: `.get()`, slice patterns, or `try_from`.

### Types

- Newtype or enum anything carrying an invariant or a unit; do not let a
  domain value decay into a raw `String`/`i32` at a boundary (`ActionType`,
  `OutletType`).
- Fields stay public on serde and DB DTOs; private fields only when a
  constructor has to enforce something.
- Prefer `Option<T>` over sentinel values. Validate at the trust boundary,
  not in every caller.

### Functions

- One job each; return early instead of nesting.
- Prefer borrowing (`&T`, `&[T]`); take ownership only when the value is
  stored. Use `Cow<'_, str>` when ownership is conditional.
- Keep it around five parameters; beyond that pass a small struct. The LLM
  batch call is the one known exception, marked `#[allow(clippy::too_many_arguments)]`.

### Idioms

- Iterators when they read better than a loop; `enumerate()` over a manual
  counter.
- Explicit `.clone()` on non-`Copy` types; no hidden clones in closures.
- Exhaustive `match`; no `_` catch-all on a local enum. External input
  (JSON, arbitrary strings) is the exception.
- Anything whose ignored value would be a bug carries `Result` or
  `#[must_use]`.
- `&str` in signatures, `Vec::with_capacity` only for known sizes.

### Imports

- No glob imports; `use super::*` only inside `#[cfg(test)]` modules.
- Order: `std`, external crates, `crate::`.

### Tests

- `#[cfg(test)]` module per file, with its body in a sibling `<module>_test.rs`
  via `#[path]`; Arrange-Act-Assert, failure messages that say what broke.
- Tests that only use the public API go in `tests/` as standalone
  integration-test crates.
- `mod.rs` files only declare submodules and re-export them; implementation
  lives in the named child module.
- Every new branch, parser, or dedup/match rule gets a test; trivial
  pass-throughs do not.
- No network or DB in unit tests: test the pure function or mock at the
  boundary.
- HTML parsing results are `insta` snapshots driven by
  `<module>/fixtures/*.html`; add a case by dropping in a file, then
  regenerate with `INSTA_UPDATE=always cargo test` and commit the `.snap`.

