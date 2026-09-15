# Phase 6f summary — the staged-candidate conditional write's honest landing on Trino

## Shipped

- **`staged_relation_atomicity` census fix.** `external_test_module_files` pre-scans
  `crates/smelt-runtime/src/**/*.rs` for `#[cfg(test)] mod <ident>;` declarations and resolves
  them to `<dir>/<ident>.rs`/`<dir>/<ident>/mod.rs`, excluding those files from the production
  scan. `cumulative/tests.rs` (the `0e1ab3e12` split) no longer counts. New test
  `census_still_flags_a_production_file` proves the exclusion is scoped — a synthetic tree with
  both an excluded test-module file and a genuine production hit still flags the production file.
  6e's row flipped to `done` (its own scope was already complete).
- **Spec delta.** `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend"
  states, timelessly, that on a ledger-less backend a membership-sensitive keyed model's
  staged-candidate conditional recompute is subsumed by its own `KeyedFold` whole-target-rebuild
  downgrade — applying equally to Trino and Spark.
- **Test 3 retargeted.** `staged_candidate_conditional_parity_on_trino` →
  `membership_model_whole_target_rebuild_parity_on_trino`: asserts the one group that actually
  executes (whole-target rebuild, byte-identical to `emit_create_table_as`), mirroring
  `additive_keyed_fold_downgrade_parity_on_trino` — also the live regression guard for 6d's
  double-dispatch fix.
- **Test 4 (new): `staged_candidate_conditional_emitter_executes_on_trino`.** A direct-emitter
  live harness — `emit_staged_candidate_conditional_recompute(..., MaintenanceDialect::Trino)`'s
  statements executed against a real `TrinoBackend`, checked against a hand-computed oracle
  (departed-row delete, changed-row update, unchanged-row suppression, new-row insert). This is
  criterion 5's surviving proof for the family, independent of which model shapes route to it.
- **Test 5 (new): `column_scoped_cell_is_suppressed_by_a_whole_target_rebuild`.** Measured 6d's
  second finding (the `column_scoped_cell` vs `whole_target_rebuild_downgrade` dispatch) is
  **unreachable**, not a live bug — see Decisions. Asserts and documents the unreachability as a
  regression guard.

## Decisions

- Option (c) taken for 6d's residue (retarget + direct-emitter harness), per the plan's rejection
  of (a) (see outcome.md's decision log — threading `technique_overrides` through the downgrade
  resolvers re-homes ~100 lines of pin-validation plumbing to reach a route Trino structurally
  never takes) and (b)'s prior impossibility finding.
- 6d's dispatch-symmetry concern did NOT need the same fix 6d applied to membership recompute.
  Measured: `resolve_live_column_scoped_cell` never resolves `Technique::ColumnScopedMerge` for a
  source whose enrichment join shares a scope with a fold's `GROUP BY` —
  `skeleton_source_closure`'s v1 scope restriction refuses to prune membership sensitivity there,
  so the source always lands in `membership_sensitivity` and the cell is always `DeleteInsert`.
  Full reasoning and code citations in outcome.md's decision log and the test's doc comment.

## For the next planner

- The unreachability finding is worth a structural note if `skeleton_source_closure`'s v1 scope
  restriction is ever relaxed (it is explicitly versioned "v1") — `column_scoped_cell_is_suppressed_by_a_whole_target_rebuild`
  will start failing at that point and is the intended trip-wire.
- Phase 7 (structural no-authoring leg + no-second-derivation-site check) is next per the table.

## Gates

- `cargo test -p smelt-runtime --test staged_relation_atomicity` — 6/6 green.
- `cargo test -p smelt-runtime --test availability_seam` — 12/12 green.
- `cargo test -p smelt-runtime --test staged_relation_atomicity --test execute_parity --quiet` — green.
- `cargo clippy -p smelt-runtime --tests --quiet` — clean.
- **Live tier** (`scripts/trino-up.sh` / `scripts/trino-env.sh`):
  - `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` — 47/47 green.
  - `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 18/18 green.
  - Torn down with `scripts/trino-down.sh`.
- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  shellcheck, full-workspace `cargo test`, example_diagnostics).
