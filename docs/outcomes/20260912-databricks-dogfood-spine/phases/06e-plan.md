# Phase 6e plan — `epoch_us` enters the function registry, and the full refresh reaches 16/16

## Objective

Close the last construct blocking a complete Databricks full refresh. `silver.actor_sessions`
calls `epoch_us`, which has no `BuiltinRegistry` entry at all, so it is printed verbatim to
Databricks and fails `[UNRESOLVED_ROUTINE]`, skipping three dependents. Register it — name,
classification, type and per-dialect emission from the one table, per the Function-registry
single-ownership invariant — then re-run the live full refresh to a clean 16 success / 0 failed
/ 0 skipped. This advances criterion 6 (the *same model set* runs) and unblocks criteria 7 and 8,
which are both stated per model.

## Spec delta

None. `epoch_us` is a built-in SQL function, and the spec does not enumerate built-ins; its
per-dialect spelling is registry *data* under the rule already written in
`docs/specs/architecture.md` §"Constraints & Invariants" item 14. If the dialect-audit probe
lands a new row, `docs/reference/dialect-coverage.md` regenerates as generated output, not a
spec edit.

## Tests

Red-green, in this order:

1. `crates/smelt-types/src/signatures/tests.rs` — `epoch_us_is_registry_backed`: the registry
   resolves `EPOCH_US` as a scalar returning `BigInt` over one `Timestamp` argument.
2. `crates/smelt-db` `registry_consistency::every_recognized_function_is_registry_backed`
   (existing gate) — must stay green, i.e. `SqlFunction::from_name("epoch_us")` resolves and the
   registry entry is a recognised function. No new test; it fails loudly if only one side lands.
3. `crates/smelt-dialect` printer test `epoch_us_emits_unix_micros_on_spark`: `epoch_us(ts)`
   prints as `epoch_us(ts)` on DuckDB and `unix_micros(ts)` on SparkSQL.
4. `crates/smelt-db` type-inference test `epoch_us_infers_bigint`: a projection of
   `epoch_us(event_ts)` infers `BigInt`, not `Unknown`.
5. `crates/smelt-cli/tests/github_activity_databricks.rs::actor_sessions_compiles_without_epoch_us`
   — compiles `silver.actor_sessions` for `--target databricks --dry-run` (no workspace) and
   asserts no bare `epoch_us(` survives in the emitted SQL.

## Tasks

1. Add `SqlFunction::EpochUs` — variant in `crates/smelt-types/src/functions/mod.rs` (enum + the
   `ALL` list), spelling `"EPOCH_US"` in `name.rs`, scalar arm in `category.rs`.
2. Add the `EPOCH_US` `Signature` to `crates/smelt-types/src/signatures/builtins/extended_temporal.rs`:
   one `Timestamp` argument, `BigInt` return, `.with_emission(&[(DialectId::SparkSql, Position::Any,
   Emission::Template("unix_micros({0})")), (DialectId::BigQuery, Position::Any,
   Emission::Template("UNIX_MICROS({0})"))])`. DuckDB keeps the registry's default (native name).
   Comment the Spark spelling with why (`unix_micros` is Spark/Databricks' own microsecond epoch).
3. Run `cargo test -p smelt-db --test dialect_audit` and satisfy whatever coverage totality asks:
   if the derived probe needs an argument spelling, add a `spell_args("EPOCH_US", …, &["d_ts"])`
   entry in `crates/smelt-db/tests/dialect_audit/overrides.rs`; if a live leg diverges, register
   it in `ledger.rs` with a reason — never widen a ratchet.
4. Absorb inference churn: `epoch_us` previously inferred `Unknown`, so snapshots and parity
   fixtures referencing it may move. Expect `crates/smelt-cli/tests/e2e/web_analytics_refactor_snapshot.rs`,
   `cross_engine_types_parity.rs`, `github_activity_replay.rs`. Update expectations only where the
   new `BigInt` is *more* correct; investigate anything else rather than blessing it.
5. Regenerate `docs/reference/dialect-coverage.md` if its doc-sync gate reports drift.
6. Live: `source scripts/dbx-dogfood-env.sh`, then re-run the full refresh
   (`smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start 2026-08-05
   --event-time-end 2026-08-07`) and record the run id and the success/failed/skipped counts.
   **If `scripts/dbx-dogfood-env.sh` cannot reach the workspace, emit `<<PHASE_BLOCKED>>` — do not
   report the offline gates as phase completion.**
7. Write `phases/06e-summary.md`: what the counts are, and — if still short of 16/16 — the next
   failing model's exact error, recorded not fixed, per the outcome's own boundary.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full test,
  example_diagnostics).
- `cargo test -p smelt-types --test registry_coverage --quiet`
- `cargo test -p smelt-db --test integration registry_consistency --quiet`
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --quiet`
- `cargo test -p smelt-db --test dialect_audit --quiet` (DuckDB legs run in-process)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --quiet`
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --quiet`
- Live: the full-refresh run above, counts recorded in the summary.

## Commit message

`outcome(databricks-dogfood-spine): phase 6e registers epoch_us with a Spark emission spelling`
