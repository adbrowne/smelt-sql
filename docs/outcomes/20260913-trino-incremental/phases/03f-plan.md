# Phase 3f plan — every partition literal the windowed-keyed driver emits goes through the single renderer

## Objective

Close gap 4: the windowed-keyed-maintenance driver renders its own partition-value
literals typed against the referenced column, through 3b2's single `partition_literal`
owner, instead of hardcoding `PartitionColumnType::Undeclared` and raw-quoting. Two
sites of the class live here — the **driving-source pushdown filter**
(`cumulative.rs`'s `SourceBound` + `driver.rs`'s `driving_steps` `TimeRange`) and the
**target-scan slice bounds** (`TargetSlicePredicate::Range` in both keyed-fold
emitters plus `emit_recurrence_bound_probe`'s `slice_lower`). Serves success criteria
2 and 3 by unblocking the whole-row `MERGE` upsert family's first failure on Trino
(`Cannot apply operator: date <= varchar(10)` / `bigint <= varchar(10)`, measured live
in 3d); 3h re-attempts the live leg on top of this plus 3g.

## Scope note

Gap 5 (`Technique::KeyedFold` has no plan-time downgrade) is **3g's**, and it is hit
before any real Trino `MERGE` is reached. So this phase's proofs are **offline,
statement-level** — the emitted SQL's literal spelling — plus a no-regression re-run
of the existing live tier. Do not try to add a new live keyed-fold test here; it
cannot pass until 3g lands.

## Spec delta (first)

`docs/specs/incremental_shapes.md` §"The partition grain" rule 8a — the sentence
enumerating where a partition literal is rendered in the column's own type ("the
output clamp, the per-source scan filter, and the maintenance region's `DELETE`
predicate") is extended to name the remaining two sites, so the enumeration matches
the single-owner claim it already makes: **the windowed-keyed driver's per-step
driving-source pushdown filter**, and **a keyed merge's target-scan slice bound**
(and the recurrence probe that reuses the same bound). No behavioural surface changes
beyond making the already-stated rule true at these sites.

## Tests (red-green)

Offline, in `crates/smelt-runtime/tests/` unless noted.

1. `driving_source_pushdown_renders_typed_date_literal` (new file
   `keyed_driver_literal_typing.rs`) — a keyed model whose driving source declares a
   `DATE` partition column: the per-step pushed-down SQL contains `DATE '2026-01-01'`,
   not `'2026-01-01'`.
2. `driving_source_pushdown_renders_bare_quoted_for_declared_varchar` — same shape, a
   declared `VARCHAR` calendar-shaped column: spelling stays `'2026-01-01'`
   (the 3b2 regression `statement_parity`'s `staged_candidate_conditional` caught).
3. `driving_source_pushdown_renders_bare_integer_on_integer_axis` — an integer-axis
   driving source pushes `>= 7`, never `>= '7'`.
4. `keyed_fold_target_slice_renders_typed_bounds` (`smelt-logical`, extend
   `crates/smelt-logical/tests/emit_statements*`) — `emit_keyed_fold` and
   `emit_keyed_fold_suppressed` with a `Range` slice on a `Date` column emit
   `BETWEEN DATE '…' AND DATE '…'`; on `Text`/`Undeclared` the spelling is unchanged
   byte-for-byte from today.
5. `recurrence_bound_probe_renders_typed_lower_bound` (same file) —
   `emit_recurrence_bound_probe`'s `target.<col> < …` bound takes the same typed
   spelling from the same renderer.
6. `no_maintenance_emitter_quotes_a_partition_value_by_hand` (gate, in
   `smelt-logical`'s existing emitter test binary) — source scan over
   `crates/smelt-logical/src/maintenance/emit/` and
   `crates/smelt-runtime/src/maintenance_driver/` finds no `'{`-style inline quoting
   of a slice/partition bound; every such bound must be a `partition_literal` result.
   Keep the pattern list explicit and narrow so the gate fails loudly, not vaguely.
7. Regression: `locality_route1_slice_pruning`, `locality_route2_derived`,
   `locality_route3_recurrence_check`, `probe_dispatch`, `statement_parity`,
   `dry_run_statements` all still pass (signature changes touch their call sites).

## Tasks

1. Edit rule 8a in `docs/specs/incremental_shapes.md` per the spec delta above.
2. Add `column_type: PartitionColumnType` to `TargetSlicePredicate::Range`
   (`smelt-logical/src/maintenance/emit/merge.rs`); render `lower`/`upper` via
   `partition_literal(PartitionAxis::Calendar, column_type, …)` in both
   `emit_keyed_fold` and `emit_keyed_fold_suppressed`, propagating the renderer's
   `Err` rather than swallowing it (fail-loud).
3. Add a `column_type` parameter to `emit_recurrence_bound_probe` and render
   `slice_lower` through the same owner.
4. Add a `column_type` parameter to `driving_steps` (`maintenance_driver/driver.rs`)
   and use it for the `MaintenanceStep`'s `TimeRange.column_type`; remove the
   hardcoded `Undeclared` at `driver.rs:66`.
5. Thread the driving source's declared type into `cumulative.rs`: add a
   `source_infos: &[smelt_core::SourceInfo]` parameter to
   `execute_cumulative_aggregate`, resolve the driving source's column type once via
   `crate::execute::sources::source_partition_column_type` (the existing single
   owner — do not re-implement), and use it for both the `SourceBound.column_type`
   at `cumulative.rs:557` and the `driving_steps` call.
6. Resolve the **model's own** partition-column type once in `cumulative.rs` via
   `compiler.get(target).resolve_partition_column_type(...)` (the 3b2 owner) and
   thread it into the `TargetSlicePredicate::Range` construction in
   `driver.rs`'s `run_windowed_keyed_maintenance` (pass it in alongside the existing
   `locality` argument rather than re-deriving inside the driver).
7. Pass `source_infos` at the single caller
   (`execute/project/mod.rs:~1953`, already in scope).
8. Repair every call site the two signature changes touch (tests listed above),
   keeping `Undeclared` only where no type is genuinely resolvable.
9. Add the tests; run the offline gates, then the live tier for no-regression.
10. Update `.claude/large-file-baseline.txt` with `--update` only if a file genuinely
    grew; note it in the summary as the sign-off record.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage`
- `cargo test -p smelt-runtime --test keyed_driver_literal_typing --test
  locality_route1_slice_pruning --test locality_route2_derived --test
  locality_route3_recurrence_check --test probe_dispatch --test statement_parity
  --test dry_run_statements --test execute_parity --test partition_axis_windowing`
- `cargo test -p smelt-cli --test maintenance_conformance --test transformer_metamorphic`
- Live tier no-regression (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`):
  `cargo test -p smelt-backend-trino` and `cargo test -p smelt-cli --test
  trino_incremental_families --test trino_ddl_live --test trino_state_residency`;
  `bash scripts/trino-down.sh`. If the coordinator is unreachable, emit
  `<<PHASE_BLOCKED>>` — never report the live leg green when it skipped.

## Commit message

`fix(maintenance): render the windowed-keyed driver's partition literals through the single typed renderer`
