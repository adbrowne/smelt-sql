# Phase 3 summary — `MaintenanceDialect::Trino` landed; three pre-existing cross-cutting bugs found and routed around

## Shipped

- `MaintenanceDialect::Trino` exists (`crates/smelt-logical/src/maintenance/emit/types.rs`) and
  `smelt_backend::maintenance_dialect(SqlDialect::Trino)` now returns `Ok` (`crates/smelt-backend/src/lib.rs`).
  Every match site the new variant forced now has an explicit arm, each classified per the
  three-way discipline (executed-and-measured / later-phase-spelled / refused-by-Result) in a doc
  comment: `merge.rs`, `hash.rs`, `bootstrap.rs`, `fingerprint.rs`, `partition_bucket.rs`,
  `probes.rs`, `succession/mod.rs`, `sidecar.rs`.
- `merge.rs`'s whole-row upsert emitters (`emit_column_scoped_merge`, `emit_column_scoped_merge_suppressed`,
  `emit_keyed_fold`, `emit_keyed_fold_suppressed`) spell Trino's Iceberg `MERGE` column-by-column on
  both arms (`whole_row_update_set`/`whole_row_insert_arm`, now column-list-aware on every dialect
  that needs one) — matches phase 1's measured grammar exactly, unit-tested.
- `null_safe_eq` takes Trino's `IS NOT DISTINCT FROM` arm; `require_merge_columns` extends its
  empty-column-list refusal to Trino (same shape as BigQuery's).
- `TrinoBackend::insert_into_from_query` implemented (`INSERT INTO <table> <select>`).
- `succession/mod.rs`'s `check_succession_dialect` refuses Trino by name (`SuccessionPatch`
  downgrades to `DeleteInsert`, per T3's ruling) — unit-tested alongside Spark's existing refusal.
- **Live-verified, three ways** (`crates/smelt-backend-trino/tests/backend_live.rs`, all pass against a
  real Trino/Iceberg tier):
  - `insert_into_from_query_appends_and_leaves_prior_rows_intact` — the append family's write primitive.
  - `delete_and_insert_transactional_covers_two_disjoint_windows` — the append family's region
    DELETE+INSERT, two disjoint integer-axis windows, union-correct.
  - `merge_into_upserts_matched_and_unmatched_rows_across_two_runs` — the whole-row MERGE upsert,
    matched update + unmatched insert + idempotent-by-key across two runs with a mutation.
- A real bug found and fixed live: `hash_digest_expr`'s Trino arm originally emitted raw
  `sha256(to_utf8(expr))` (`varbinary`), which broke the digest-of-digests composition
  (`fingerprint.rs`'s `column_fingerprint_expr`/`row_fingerprint_expr`, which concatenate one
  digest straight into the next). Fixed to hex-encode (`to_hex(sha256(to_utf8(expr)))`), matching
  DuckDB's and Spark's own hex-string digest shape — `hash_digest_expr` and `hash_hex_expr` are now
  identical on Trino, exactly as they already are on the other two dialects.
- Spec delta landed: `docs/specs/multi_backend.md` — deleted the "No maintenance dialect on Trino"
  divergence entry; restated the `frozen_horizon` probe clause (renders like any other dialect now).
- Re-pointed the tests the `Ok(...)` flip forced: `smelt-backend`'s
  `maintenance_dialect_is_ok_for_all_four_dialects`, `staged_relation_atomicity.rs`,
  `trino_contract_points.rs` (declared `frozen_horizon` on Trino now resolves, no warning),
  `trino_explain_downgrade.rs` (show-sql/dry-run no longer abort; `dry_run_on_trino_renders_
  delete_insert` is now a positive assertion — the region DELETE+INSERT text actually renders).

## Decisions

- **The two new CLI-level `execute_project` tests (`trino_incremental_families.rs`, test 7 of the
  plan) were not achieved as originally scoped.** Instead the family proofs moved to the
  `Backend`-trait level in `backend_live.rs` (see "Shipped" above) — same statement text, same
  `MaintenanceDialect`, invoked directly rather than through the CLI's window-parsing/cell-derivation
  layers. `trino_incremental_families.rs` is now a documentation-only file naming the three blocking
  discoveries below, each with an `#[allow(dead_code)] fn gap_N_...()` anchor rather than a live test,
  so the finding has a discoverable, greppable home instead of living only in prose.
- **`statement_parity/trino.rs` (test 8) was not built.** It depends on a real `execute_project`
  run for both families, which hits the same gaps as the CLI tests above.
- **2026-09-14 — three pre-existing, cross-cutting bugs found live, none owned by this phase, all
  masked on every other backend by looser implicit-coercion rules:**
  1. **Calendar-literal type coercion.** `partition_literal`'s calendar axis (`smelt-logical`) and
     `transformer.rs`'s injected scan-window predicate (`smelt-runtime`) both render a bare quoted
     string (`'2026-01-01'`) compared against a `DATE`/`TIMESTAMP` column. DuckDB/Spark/BigQuery
     implicitly coerce it; Trino refuses (`Cannot apply operator: date <= varchar(10)`). Fix shape:
     an ANSI `DATE '...'`/`TIMESTAMP '...'` literal works on all four engines, but changing
     `partition_literal`'s output format is a global byte-format change ~19 files pin exact text
     against — needs its own reviewed phase, not a phase-3-scoped fix.
  2. **Real (non-dry-run) execution does not re-resolve a bare-integer run window's axis.**
     `execute/window.rs::parse_run_window` returns `(None, None)` for a bare-integer bound pair (by
     design, serving only calendar-only consumers); the `(None, None)` dispatch arm in
     `execute/project/mod.rs` then builds the model's `TimeRange` with a **hardcoded calendar axis**
     rather than re-resolving the model's own axis the way `parse_run_window_in_axis` does for
     `--dry-run`/`build_model_plans`. Result: a genuine integer-axis model's real run injects
     `batch_id >= '1'` — quoted, against an `INTEGER` column. DuckDB tolerates it (implicit cast);
     Trino refuses. `partition_residue_probes.rs::probe_integer_partition_column_run` exercises the
     identical model shape today, only against DuckDB, so it has never caught this.
  3. **A snapshot-reconcile-shaped keyed model's `ColumnScopedMerge` downgrade assumes a `ScanClamp`
     that cannot exist for it.** The natural "whole-row MERGE upsert" shape (`refresh: incremental`,
     `grain: key`, a plain `ANY_VALUE` passthrough of an unclocked `mutable_snapshot` source, no
     upstream model edge) does **not** dispatch through `cumulative.rs`'s `execute_snapshot_
     reconcile` — it goes through `smelt_logical::maintenance::choice`'s general cell derivation,
     whose ideal technique is `ColumnScopedMerge` (confirmed via `smelt explain --json`'s
     `state_downgrade.original`). On a fully-degraded dialect that downgrades to `PerGroupRecompute`,
     whose repair-driver execution assumes a derivable `ScanClamp` — sound for a clocked, windowed
     cell, but this cell's trigger is `UpstreamMutation`, not a clock, so no clamp exists to derive.
     Execution refuses with `MaintenanceRepairSliceMissing` instead of falling back to the full-scan
     recompute the `key_scope: None` "reachable" row already promises. `20260913-trino-ledger`'s
     Spark twin realises the identical fully-degraded posture, so this is very likely reachable on
     Spark too — not Trino-specific, and it lives in `smelt_logical::maintenance::choice`'s
     cell-derivation layer, not in this phase's `MaintenanceDialect::Trino` emitters.

## For the next planner

- **The three gaps above block phase 3's own stated Tests 7 and 8 and criterion 2's full
  `execute_project` proof.** Each is real, reproducible, and documented with the exact live error
  text in `trino_incremental_families.rs`'s doc comments. None is Trino-specific in mechanism (gap 1
  and 2 are literal-typing bugs that would affect Spark/BigQuery/DuckDB identically if those engines
  were as strict; gap 3 explicitly affects Spark's identical fully-degraded posture) — Trino's
  strictness is simply what surfaced them. Recommend a dedicated fix phase (or three) before the
  remaining phases (4-10) attempt further live `execute_project` proofs, since phases 4-6 (delete-
  and-insert window, merge-less conditional write, degraded routes) will very likely hit gaps 1-3
  again for any calendar-partitioned or snapshot-reconcile-shaped model.
- Gap 2's fix is narrow and mechanical once someone signs off on it: make the `(None, None)` arm in
  `execute/project/mod.rs` call `parse_run_window_in_axis` (already exists, already used by
  `build_model_plans`) instead of hardcoding `PartitionAxis::Calendar`.
- Gap 3's fix belongs in `smelt_logical::maintenance::choice`'s downgrade derivation: a
  `PerGroupRecompute` cell downgraded from `ColumnScopedMerge` for an `UpstreamMutation`-triggered
  (not clock-triggered) cell should resolve `key_scope: None` (the always-full-scan route) rather
  than whatever key-scope classification currently produces a `ScanClamp` requirement.
- Gap 1 is the largest: it needs a design decision (ANSI literal everywhere and eat the ~19-file
  pinned-text rewrite, vs. per-dialect literal rendering threaded through `partition_literal`/
  `Region`/`inject_time_filter`/`inject_source_filters`) before a plan can be written against it.
- Everything phase 3 actually owns (the `MaintenanceDialect::Trino` mapping and every emitter arm it
  forced) is done, live-verified, and should not need revisiting when the above gaps are fixed —
  the three `backend_live.rs` tests will keep passing unchanged.

## Gates

- `cargo fmt --all -- --check` — pass
- `bash .claude/scripts/clippy-gate.sh` (both feature sets) — pass, zero warnings
- `bash .claude/scripts/large-file-check.sh` — pass (fingerprint.rs baseline bumped 1196→1202,
  sign-off note added)
- `cargo test -p smelt-logical --quiet` — pass (1006+ tests)
- `cargo test -p smelt-backend --quiet`, `--test merge_columns_guard` — pass
- `cargo test -p smelt-runtime --quiet` — pass (all suites)
- `cargo test -p smelt-cli --quiet` — pass (full suite, 0 failures)
- `cargo test -p smelt-cli --test example_diagnostics` — pass (129 passed, 1 ignored)
- Live (`scripts/trino-up.sh` / `source scripts/trino-env.sh`):
  - `cargo test -p smelt-backend-trino --test backend_live` — pass, including all 3 new tests
  - `cargo test -p smelt-cli --test trino_incremental_spec_freshness --test trino_emission_spec_freshness --test trino_explain_downgrade --test trino_ci_wiring --test trino_posture_plan_invariance` — pass
- **Not achieved**: `trino_incremental_families.rs` as a live `execute_project`-driven CLI test, and
  `statement_parity/trino.rs` — see "Decisions" and "For the next planner" above.
