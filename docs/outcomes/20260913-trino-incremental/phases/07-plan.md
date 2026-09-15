# Phase 7 — the structural no-authoring leg for Trino, and the plan-derivation census

## Objective

Close the two *structural* claims the outcome still asserts by convention rather than by gate:
criterion 5's second half (`statement_parity`'s no-authoring leg must actually **scan**
`smelt-backend-trino`, which it does not today — the crate list stops at DuckDB/Spark) and
criterion 6 in full (the maintenance plan is derived once, and no plan-consuming code branches on
`SqlDialect::Trino`). Both are offline source-scan gates in the style of
`hardening_budget.rs`/`staged_relation_atomicity.rs`; no live tier is required for anything in this
phase.

## Spec delta

Not a user-visible behaviour change, but it retires a stated convention, so the invariant text moves
with it:

- `CLAUDE.md`, maintenance-plan purity bullet — replace "Upheld by convention for the
  plan-derivation half (structural assertion tracked in `docs/plans/20260707-maintenance-plan-impl.md`)"
  with the standing gate this phase lands (`cargo test -p smelt-runtime --test statement_parity`,
  plan-derivation census legs).
- `docs/specs/architecture.md` §"Constraints & Invariants" item 12 — same edit, timelessly phrased:
  the single-derivation-site and no-consumer-dialect-branch halves are gated, not conventional.

## Tests

Red-green, all in `crates/smelt-runtime/tests/statement_parity/`.

In `structural_and_ledger.rs`:
1. `trino_backend_is_in_the_no_authoring_scan_scope` — the crate list
   `no_maintenance_statement_authoring_outside_the_emitter` scans contains `smelt-backend-trino`
   (red today). Hoist the list into a `const` so the gate and this test read one source.
2. `bigquery_backend_is_in_the_no_authoring_scan_scope` — same for `smelt-backend-bigquery`, the
   second unscanned backend this phase measured.
3. `a_trino_spelled_merge_in_a_backend_file_is_flagged` — a synthetic temp tree holding
   `MERGE INTO "iceberg"."s"."t" USING …` (Trino's double-quoted spelling, not DuckDB's) is flagged
   by `scan_statement_authoring_file`, proving the shape list catches Trino's own rendering rather
   than passing because the crate happens to be clean.

In a new `plan_derivation_census.rs` (declared from `main.rs`):
4. `maintenance_plan_is_derived_in_exactly_one_production_site` — a scan of every
   `crates/*/src/**/*.rs` production file for `derive_maintenance_plan`/
   `derive_maintenance_plan_with_referential_integrity*`/`append_model_edge_cells`/`derive_triggers`
   call sites; the hit file set must equal the allowlist (`smelt-logical/src/maintenance/derive/`,
   the owner, and `smelt-db/src/queries/maintenance/plan.rs`, the one Salsa-cached consumer).
5. `census_flags_a_second_derivation_site` — a synthetic file calling `derive_maintenance_plan(`
   outside the allowlist is flagged (the scoping proof for test 4).
6. `plan_consumers_hold_no_per_dialect_branch` — a scan of the plan-consuming trees
   (`smelt-db/src/queries/maintenance/`, `smelt-db/src/maintenance_refs/`, `smelt-planner/src/`,
   `smelt-runtime/src/execute/`) for `SqlDialect::`/`MaintenanceDialect::`/`BackendType::`
   occurrences; each must match an allowlist entry `(file suffix, substring, reason)`. The only
   entries are the dialect-*selection* sites measured today: `execute/targets.rs`'s two total
   `BackendType → SqlDialect` maps, `write_pin.rs`'s pin-name parse, and
   `execute/project/mod.rs`'s three pre-existing `== SqlDialect::DuckDB` capability narrowings
   (delta-restricted dispatch ×2, the DuckDB-qualified report at ~4245).
7. `no_consumer_dialect_allowlist_entry_names_trino` — criterion 6 in assertion form: no allowlist
   entry from test 6 names Trino, so adding Trino provably introduced no consumer-side branch.

## Tasks

1. Hoist `no_maintenance_statement_authoring_outside_the_emitter`'s crate list to a `const`; write
   tests 1–3 red.
2. Add `smelt-backend-trino` to the list (measured clean — no allowlist entry needed).
3. Add `smelt-backend-bigquery`, with allowlist entries for its two `Backend::delete_partitions`/
   `create_table_as` shapes (`sql.rs:108` `DELETE FROM {} WHERE {} >= {} AND {} < {}` and
   `sql.rs:32` `CREATE OR REPLACE TABLE {} AS {}`) — the same pre-existing class already
   allowlisted for DuckDB and Spark, with the same rationale comment. **If BigQuery needs an entry
   outside that class, do not widen the allowlist**: record the hit in the summary and escalate it
   to `20260906-bigquery-correctness`, per this outcome's existing BigQuery out-of-scope line.
4. Write `plan_derivation_census.rs` with tests 4–7 red, then land the allowlists so they pass;
   every entry carries a one-line reason.
5. Make the spec-delta edits to `CLAUDE.md` and `docs/specs/architecture.md` item 12.
6. Check the new file against `.claude/large-file-baseline.txt` (keep `plan_derivation_census.rs`
   well under the ratchet; it is a fresh file, so it must not be added to the baseline).

## Verification

- `cargo test -p smelt-runtime --test statement_parity` — offline legs green (the live Trino legs
  need the tier and are unchanged by this phase; if `scripts/trino-up.sh` comes up cleanly, run the
  whole binary with `--test-threads=1` as a bonus and tear down, but do not report a skipped live
  leg as green).
- `cargo test -p smelt-runtime --test execute_parity` — unchanged pipeline ownership.
- `cargo clippy -p smelt-runtime --tests --quiet` — clean.
- `bash .claude/scripts/verify-phase.sh` — full gate (criterion 11).

## Commit message

`test(smelt-runtime): scan Trino and BigQuery for statement authoring, and gate the plan's single derivation site`
