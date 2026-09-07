# Phase 1 summary — Retire the PostgreSQL emission dialect (#181)

## Shipped

- `DialectId::PostgreSql` and `SqlDialect::PostgreSQL` deleted
  (`crates/smelt-types/src/dialect_id.rs`, `crates/smelt-dialect/src/dialect.rs`).
  `DialectId::ALL` / `SqlDialect` now have exactly three variants: DuckDB, Spark, BigQuery.
- `BackendCapabilities::postgresql()` deleted.
- The three PostgreSQL registry rows removed from `crates/smelt-types/src/signatures.rs`
  (`//` Unsupported, `EXPLODE` rename, and the `needs_cast_for` test assertion).
- New durable gate `smelt-types/tests/registry_coverage.rs::no_registry_row_names_a_retired_dialect`
  — scans `signatures.rs` source for `PostgreSql`/`PostgreSQL`; stops a later template/arm phase
  from reintroducing an unverifiable column.
- New gate `smelt-db/tests/dialect_audit/main.rs::baseline_names_exactly_the_audited_dialects`
  — two-sided check that `.claude/dialect-gaps-baseline.txt` names exactly `DialectId::ALL`.
- `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_postgres` line removed with a dated
  sign-off comment naming #181 and this phase.
- `docs/reference/dialect-coverage.md` regenerated — three dialect columns, no PostgreSQL row.
- Two string-keyed PostgreSQL paths beyond the emission enums also removed (folded into this
  phase per the outcome's 2026-09-06 decision-log note): `backend_dialect_for` /
  `backend_write_capabilities_for` (`smelt-db/src/queries/maintenance.rs`) and the `"postgres"`
  branch of `smelt-logical`'s `as_struct` lowering — both were unreachable surface
  (`Target::backend_type` already rejects `type: postgres`).
- Test coverage for constructs that only had a PostgreSQL case ported to a surviving dialect
  rather than dropped: QUALIFY rewrite → Spark, EXPLODE→UNNEST rename → BigQuery, EVERY-native
  (no remap) → Spark — in both `printer.rs` unit tests and `smelt-dialect/tests/snapshots.rs`.
- Spec edits: `docs/specs/multi_backend.md` (`SqlDialect` list, backend-namespace example,
  `//`/MEDIAN/null-safe-equality prose) and `docs/specs/architecture.md` (deleted the
  "PostgreSQL emission verdicts are unverified" Known Divergences entry; narrowed the
  "Spark and PostgreSQL printers" sentence to Spark only).
- #181 was already closed (verified via `gh issue view 181`, state `CLOSED`) — no action needed.

## Decisions

- Folded the two `smelt-db`/`smelt-logical` string-keyed PostgreSQL paths into this phase
  rather than giving them their own phase, since removing unreachable surface is not a
  user-visible behaviour change (recorded in the outcome's own 2026-09-06 decision-log entry
  before this phase started).
- Where a test's only PostgreSQL case had no surviving-dialect twin already in the suite, ported
  it to the dialect the same registry row/emission fact applies to (Spark for QUALIFY-rewrite and
  EVERY-native, BigQuery for EXPLODE→UNNEST) rather than deleting the coverage outright, per the
  plan's snapshot-porting instruction.
- Left prose comments that merely describe standard-SQL syntax history (e.g. "PostgreSQL-style
  `::` cast", "PostgreSQL-only builtin" as a gap-ledger rationale, DuckDB/PostgreSQL grammar
  compatibility notes in type-inference comments) untouched — they don't name the retired
  `DialectId`/`SqlDialect` variant and aren't claims this outcome's success criteria address.

## For the next planner

- Phases 2–7 (Template emission, compile-time refusal, DuckDB/Spark gap closure, operand-conditional
  verdicts) now work against exactly three dialect columns — no template or arm row will ever need
  a PostgreSQL verdict.
- The `crates/smelt-parser-compat` pg_query grammar anchor and `docs/ROADMAP.md` were left
  completely untouched, confirmed via `git diff --stat crates/smelt-parser-compat docs/ROADMAP.md`
  (empty output).
- Nothing new surfaced outside this phase's task list; no follow-up TODO added.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt-check, clippy zero-warnings both
  feature sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-db --test dialect_audit` — 51 passed.
- `cargo test -p smelt-dialect --test emission_ownership --test snapshots --test capability_conformance` — 45 passed.
- `cargo test -p smelt-types --test registry_coverage` — 83 passed.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity` — 16 passed.
- `cargo test -p smelt-db --test integration registry_consistency` — 6 passed.
- `git diff --stat crates/smelt-parser-compat docs/ROADMAP.md` — empty (untouched), as required.
