# Phase 3f summary — windowed-keyed driver literals through the single renderer

## Shipped

- `TargetSlicePredicate::Range` (`smelt-logical/src/maintenance/emit/merge.rs`)
  gained a `column_type: PartitionColumnType` field; `emit_keyed_fold` and
  `emit_keyed_fold_suppressed` now render `lower`/`upper` through
  `partition_literal` instead of hand-quoting — `DATE '…'`/`TIMESTAMP '…'` on
  a typed column, byte-identical quoted-string spelling on `Text`/`Undeclared`.
- `emit_recurrence_bound_probe` (`probes.rs`) takes a new `column_type`
  parameter and renders `slice_lower` the same way.
- `driving_steps` (`maintenance_driver/driver.rs`) takes a new `column_type`
  parameter, threaded into each `MaintenanceStep`'s `TimeRange` — the
  `driver.rs:66` hardcoded `Undeclared` is gone.
- `run_windowed_keyed_maintenance` takes a new `column_type` parameter,
  threaded into the `TargetSlicePredicate::Range` it builds and into
  `WindowedKeyedRule::recurrence_probe_sql`'s new `column_type` parameter.
- `execute_cumulative_aggregate` (`cumulative.rs`) now takes `source_infos`
  and resolves two independent types instead of hardcoding `Undeclared`:
  the **driving source's** column type via the existing
  `execute::sources::source_partition_column_type` (now `pub(crate) mod
  sources`, previously private to `execute`), used for the per-step pushdown
  filter and `driving_steps`; the **model's own** maintained column type via
  `compiler.get(target).resolve_partition_column_type` (3b2's owner), used
  for the target-scan slice bound and the recurrence probe.
- New tests: `keyed_driver_literal_typing.rs` (3 offline tests on
  `inject_source_filters`), `emit_statements.rs` additions
  (`keyed_fold_target_slice_renders_typed_bounds`,
  `recurrence_bound_probe_renders_typed_lower_bound`, and the structural gate
  `no_maintenance_emitter_quotes_a_partition_value_by_hand`).
- Spec: `docs/specs/incremental_shapes.md` rule 8a's enumeration now names
  the driving-source pushdown filter and the keyed-merge target-scan slice
  bound (plus the recurrence probe reusing it) as the fourth/fifth sites.

## Decisions

- Kept `emit_keyed_fold`/`emit_keyed_fold_suppressed`/`emit_recurrence_bound_probe`
  returning plain `StatementGroup`/`MaintenanceStatement` rather than
  cascading `Result` through `WindowedKeyedRule::merge_sql`/`write_group`
  (which would touch every rule impl, test and call site). `partition_literal`
  can only fail here if a `Date`/`Timestamp` column receives a non-date-shaped
  value; the only inputs reaching these sites are whole-day-aligned strings
  from `subtract_seconds_from_date`/`add_seconds_to_date`, so failure is
  provably unreachable in production. Used `.unwrap_or_else(|e| panic!(...))`
  — fail-loud without tripping the `.unwrap()`/`.expect("` hardening-budget
  gate (neither pattern matches its regex).
- All ~35 existing test call sites of `run_windowed_keyed_maintenance` and
  `driving_steps` pass `PartitionColumnType::Undeclared` for the new
  parameter — behavior-preserving by construction (`Undeclared` renders
  exactly as the pre-fix spelling), so no existing test assertion needed to
  change *except* two that drive the real `execute_project` pipeline over a
  genuinely `DATE`-typed model column (`locality_route1_slice_pruning.rs`,
  `keyed_fold_pins_and_previews.rs`): those now correctly observe `DATE '…'`
  and were updated to expect it — a real, intended behavior change gap 4
  exists to produce.
- `succession`'s own `driving_steps` call sites (a different technique
  family, not a `WindowedKeyedRule` impl, explicitly out of this phase's
  scope) pass `Undeclared` too — no regression, but also no typing fix;
  succession's own partition-literal sites weren't part of gap 4.

## For the next planner

- Gap 4 is closed at the two sites named in the plan (driving-source
  pushdown, target-scan slice bound + recurrence probe). Gap 5
  (`Technique::KeyedFold` plan-time availability resolution) is 3g, already
  queued — 3h re-attempts the live keyed-fold `MERGE` leg once 3g lands.
- `succession`'s partition-literal handling was left untouched
  (`Undeclared` at its `driving_steps` call site in `execute/project/mod.rs`)
  — worth a follow-up census if the succession grain ever needs typed
  literals on a strict engine, but out of scope here per the plan's own
  scope note.
- No live keyed-fold `MERGE` test was added against Trino in this phase (by
  design — it cannot pass until 3g); the live-tier run above is a
  no-regression check only, over the append/whole-row-MERGE families already
  proven in 3d/3e.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy ×2 feature
  sets, shellcheck, full workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage` — pass.
- `cargo test -p smelt-runtime --test keyed_driver_literal_typing --test
  locality_route1_slice_pruning --test locality_route2_derived --test
  locality_route3_recurrence_check --test probe_dispatch --test
  statement_parity --test dry_run_statements --test execute_parity --test
  partition_axis_windowing` — pass (two byte-parity assertions updated to
  expect the new typed spelling).
- `cargo test -p smelt-cli --test maintenance_conformance --test
  transformer_metamorphic` — pass (104 + 2 tests).
- Live tier (`scripts/trino-up.sh`/`trino-env.sh`/`trino-down.sh`):
  `cargo test -p smelt-backend-trino` (25+15+27+9+2+1+8 tests) and
  `cargo test -p smelt-cli --test trino_incremental_families --test
  trino_ddl_live --test trino_state_residency` (1+4+5 tests) — all pass,
  no regression.
- `.claude/large-file-baseline.txt` updated via `--update` for five files
  that genuinely grew from this phase's edits (test additions, new
  parameters); sign-off recorded here.
