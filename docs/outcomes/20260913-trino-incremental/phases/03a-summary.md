# Phase 3a summary — real run resolves each batch's TimeRange in its own partition axis

**Shipped:**
- `crates/smelt-runtime/src/execute/project/mod.rs`: the two real bug sites — the
  per-batch `run_range` (~3376) and `scan_range` (~3391) inside the `grain: partition`
  batch loop — now build `TimeRange { axis: batch.partition_start.axis() }` /
  `{ axis: batch.scan_start.axis() }` instead of a hardcoded
  `smelt_logical::PartitionAxis::Calendar`. `batch.partition_start`/`batch.scan_start`
  are already axis-aware `PartitionPoint`s from `build_model_plans`; no second
  axis-resolution site was introduced.
- Sites audited and left `Calendar` with a one-line comment naming why (genuinely
  calendar-only by construction, per task 4): `mod.rs` ~1727 (the windowed-keyed
  driver's `time_range`, reached only when the global `parse_run_window` produced a
  calendar-shaped `(Some, Some)` — an integer pair parses to `(None, None)` there —
  and feeding `maintenance_driver/driver.rs::driving_steps`, which itself only accepts
  `%Y-%m-%d`), `mod.rs` ~2202/~2372 (bare `grain: key` `PartitionRange`s with
  `column: String::new()` — no predicate is ever rendered against `axis` since there's
  no column to filter), and `maintenance_driver/driver.rs` ~61 (`driving_steps` itself,
  which parses `start`/`end` with `NaiveDate::parse_from_str` and has no integer-axis
  form to preserve).
- `crates/smelt-runtime/tests/partition_axis_windowing.rs`: two new tests under
  `mod real_run_axis` driving a real (non-`--dry-run`) `execute_project` call —
  `real_run_batch_sql_renders_integer_axis_bounds_bare` (red before the fix: batch SQL
  quoted `batch_id >= '1'`) and `real_run_batch_sql_still_quotes_calendar_axis_bounds`
  (regression fence, green throughout).
- `crates/smelt-cli/tests/partition_residue_probes.rs::probe_integer_partition_column_run`:
  extended with row-level assertions after the backfill (exactly batch_id 1/2/3) and
  after the steady-state re-run (unchanged).
- `crates/smelt-cli/tests/trino_incremental_families.rs`: gap-2's doc-only anchor
  replaced with a real live test, `integer_axis_incremental_model_runs_on_trino` —
  seed → `--batch-size 1` backfill → steady-state re-run of an integer-`partition_column`
  model through `smelt run --target trino`, against the live Iceberg tier. Passed live.
- `.claude/large-file-baseline.txt`: `execute/project/mod.rs` baseline raised
  5098 → 5115 (comments only), sign-off note added, prior history preserved.

**Decisions:**
- The delete-and-insert `PartitionRange` at ~3684 was already correct
  (`axis: batch.partition_start.axis()`, landed earlier) — confirmed unaffected by gap 2,
  so phase 4's delete-and-insert window work inherits a working axis, not a second bug.
- The three `grain: key`/windowed-keyed sites are true calendar-only-by-construction, not
  latent integer-axis bugs — the windowed-keyed driver (`driving_steps`) has no
  integer-axis form at all today; that's a possible future extension, not this phase's gap.

**For the next planner:**
- Row 3d (phase 3's two deferred live tests — the append/whole-row-MERGE CLI proof and
  `statement_parity`'s Trino leg) is next; this phase's fix removes the specific blocker
  those tests hit (`Cannot apply operator: integer <= varchar`) for an integer-axis model,
  but 3d's own fixtures are calendar-axis, so it should be unblocked by 3b (gap 1, calendar
  literal typing) rather than by this phase.
- If a future outcome wants `refresh: keyed`/windowed-keyed maintenance on an integer
  partition axis, `driving_steps` (`maintenance_driver/driver.rs`) needs a real
  integer-step form — currently hard-refuses via `NaiveDate::parse_from_str`. Not required
  by this outcome's scope.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test partition_axis_windowing --test windowing_parity
  --test partition_grid_validation` — 9+4+29 passed.
- `cargo test -p smelt-cli --test partition_residue_probes` — 4 passed.
- `cargo test -p smelt-runtime --test execute_parity --test dry_run_statements` — 3+4 passed.
- Live tier (`scripts/trino-up.sh` / `source scripts/trino-env.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families` — 1 passed;
  `cargo test -p smelt-backend-trino --test backend_live` — 15 passed (phase 3's three
  proofs unchanged). Tier torn down afterward (`scripts/trino-down.sh`).
