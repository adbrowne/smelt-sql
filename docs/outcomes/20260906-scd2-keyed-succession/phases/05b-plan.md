# Phase 5b — Runtime dispatch of the succession cell

## Objective

Make a recognised succession model actually *run*: the window-forward driver steps the
driving source's partitions, runs the clock-tie probe read-only, and applies the phase-4
patch group (tombstone insert + presented `MERGE`) plus the merge-ledger frontier record in
one backend transaction, consuming the phase-5a `SuccessionRecipe` and never re-parsing model
SQL. Advances criterion 5 (everything except `--full-refresh`/`repair` rebuild, which is 5c)
and keeps `execute_parity` green.

## Spec delta

None. Phase 1 pinned the residual surface (`incremental_shapes.md` §"The tombstone ledger
(hidden state)", §"Run shape and late events", §"The transactional frontier write (merge
ledger)"); this phase implements already-specified behaviour and adds no new user-visible
surface. `diagnostics.md`'s `SuccessionClockTie` entry already exists.

## Design (settled here so the implementer does not re-litigate it)

- **New module `crates/smelt-runtime/src/maintenance_driver/succession/`** (`mod.rs` =
  live-cell resolution, `execute.rs` = the step loop), *not* a `WindowedKeyedRule` impl. The
  trait's seams are keyed-fold shaped (`WriteSuppression`, `KeyedWriteMechanism`,
  `emit_create_table_as` from the step's delta); succession needs a pre-write probe, a
  two-statement transactional group, and a second (tombstone) table's DDL. Widening a trait
  every keyed rule shares for one non-keyed grain is the wrong seam.
- **Reuse, don't reinvent:** `driver::driving_steps` for stepping;
  `Backend::execute_write_with_bookkeeping(ensure_sqls, pre_write_sqls, write_group)` for the
  one transaction (DuckDB wraps `pre_write` + group in a single `tx`);
  `smelt_state::ddl_duckdb::{generate_ledger_table_ddl, generate_ledger_upsert_sql}` for the
  re-run-tolerant frontier (`ON CONFLICT DO NOTHING`, never `KeyedReprocessedWindow`);
  `generate_tombstone_table_ddl` and `emit_create_empty_table` as the two `ensure_sqls`;
  `crate::probes::dispatch_probe` for the clock-tie probe.
- **Uniform patch path for every window, including the first.** Bootstrap the presented table
  as an empty shell from `UpstreamSchemas.models[<model name>]` (name + `DataType`, exactly as
  `execute::bootstrap::bootstrap_self_ref_empty_target` resolves them) and the tombstone table
  from the same list restricted to `recipe.key_cols ++ [clock_col]`. No special first-window
  branch — refold and either-order convergence are then structurally the same code path.
- **`recipe.source_table` resolution** (5a's open item): the classifier's comparison spelling
  is mapped to the physical name via `source_infos` + `db_name_for_target(model_target, schema)`,
  the same lookup the repair arm at `project.rs:~1856` uses. Unresolvable → refuse by name.
- **Window predicate**: `<source partition_column> >= '<step.start>' AND < '<step.end>'`, built
  from the driving source's own `timeseries.partition_column` (`source_timeseries`) — the run
  axis, which for an arrival-partitioned source is deliberately *not* `recipe.clock_col`.
- **Dispatch site**: `crates/smelt-runtime/src/execute/project.rs`, immediately after the
  `plan_is_keyed` block and *before* the `plan.incremental` match, gated on
  `metadata.resolved_grain().is_none()` and a resolved cell whose technique is still
  `Technique::SuccessionPatch` (a state-downgraded cell has technique `FullRefresh` and falls
  through untouched). No run window → refuse naming both `--event-time-*` flags, mirroring the
  keyed branch's own refusal. Keep the project.rs delta to the guard + one call; the ratchet is
  already red on this file.
- **`SuccessionClockTie`**: probe verdict `Violated` → `bail!` naming key columns, the clock
  column, `violation_count` and `sample_keys`, before any write.

## Tests (red-green)

Unit / driver-level (`crates/smelt-runtime/src/maintenance_driver/succession/tests.rs`,
declared `#[cfg(test)] mod tests;` so phase 3b's gate-selection rule sees it as test code):

1. `first_window_bootstraps_shell_then_patches` — empty presented + tombstone tables exist and
   the first window's rows land through the `MERGE`, not a `CREATE TABLE AS`.
2. `refolding_one_window_is_byte_identical` — apply window W twice; presented rows and ledger
   rows unchanged, run succeeds (no `KeyedReprocessedWindow`).
3. `two_windows_converge_in_either_order` — W1 then W2 equals W2 then W1, both equal to the
   model SQL at full refresh over W1∪W2.
4. `delete_event_lands_in_tombstone_ledger_not_presented` — a `is_deleted` row writes `(k, t)`
   to the ledger, is absent from the presented table, and still splices its neighbours.
5. `clock_tie_refuses_before_any_write` — a non-identical second row at the same `(k, t)`
   bails with `SuccessionClockTie` naming key/clock/sample; presented and tombstone tables are
   byte-identical to their pre-run state.
6. `identical_represented_row_is_a_no_op` — same `(k, t)` with identical content and flag: no
   refusal, no row change.
7. `failed_merge_rolls_back_the_tombstone_insert` — a recipe whose `MERGE` fails (payload
   column absent from the presented table) leaves the tombstone table row count unchanged.
8. `frontier_record_is_written_per_window` — the merge-ledger table carries one row per applied
   window, keyed on the model and the step's partition value.
9. `state_downgraded_cell_is_not_dispatched` — `StateAvailability::none()` → resolver returns
   `None`, so the model takes the ordinary full-refresh path.

Integration (`crates/smelt-runtime/tests/technique_lowering/succession_patch_e2e.rs`, harness
copied from `column_scoped_merge_e2e.rs`, over a scratch fixture project with an arrival-
partitioned `append_only` `customer_changes` source and a `customer_history` model):

10. `succession_model_runs_through_execute_project` — two `execute_project` runs over
    consecutive windows produce exactly the model SQL's own full-refresh result.
11. `late_event_in_a_later_arrival_window_splices` — an old event time arriving in a new
    arrival window repairs its neighbours' `valid_to`.

## Tasks

1. Add `succession/mod.rs` with `SuccessionCell { recipe, presented_table, source_table,
   partition_column }` and `resolve_live_succession_cell(...)` reading
   `MaintenancePlanResult.succession_recipe` + the cell's technique + `StateAvailability`.
2. Add `succession/execute.rs`: `execute_succession_maintenance(backend, model_name, schema,
   table, steps, cell, columns, retry, probe_policy, reporter, run_id) -> Result<ExecutionResult>`
   — ensure DDL, per-step event-delta `SELECT`, clock-tie probe, patch group + ledger upsert
   through `execute_write_with_bookkeeping`.
3. Report every emitted group via `reporter.maintenance_statements` (5c's `statement_parity`
   executed-vs-emitted leg depends on this being wired now).
4. Wire the dispatch guard + call into `execute/project.rs`; record the model outcome with the
   same `ModelSuccess`/manifest shape the keyed arm uses.
5. Write the fixture project under `crates/smelt-runtime/tests/fixtures/succession/`.
6. Write tests 1–11, red first.

## Verification

- `cargo test -p smelt-runtime --test technique_lowering succession` (new e2e legs)
- `cargo test -p smelt-runtime --lib maintenance_driver::succession`
- `cargo test -p smelt-runtime --test execute_parity`
- `cargo test -p smelt-runtime --test statement_parity` (structural no-authoring leg must stay
  green — the new module authors no SQL of its own)
- `cargo test -p smelt-logical --test walk_coverage`
- `bash .claude/scripts/verify-phase.sh`
- `bash .claude/scripts/large-file-check.sh` — report only; the loop's shrink step owns it.

## Commit message

`feat(smelt-runtime): dispatch succession-patch cells through the window-forward driver`
