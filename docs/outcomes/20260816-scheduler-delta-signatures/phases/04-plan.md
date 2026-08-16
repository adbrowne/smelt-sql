# Phase 4 plan — dispatch composition in the run loop

**Outcome:** `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`
**Spec:** `docs/specs/incremental_models.md` §"Dispatch — from propagated components to run
units" (already normative; landed in phase 1)
**Advances:** success criteria 1 (key-addressed dispatch outside `grain: key`, now for a model
with more than one component) and 6 (the divergence bullet's composed-multi-component residue).

## Objective

Lift phase 2's single-edge substitution gate to a **coverage** gate: a model whose every inbound
ref is a key-addressed model edge that resolved a cell dispatches *each* of those cells in one
tick, instead of falling back to the ordinary whole-model route as soon as a second edge appears.
Where coverage genuinely fails (a declared source or a non-key-addressed model edge the cells do
not restrict), the widening to the ordinary route stays — but stops being silent, per §"Widen-
never-narrow at dispatch" ("an explain-visible downgrade, never to nothing and never silently").

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences, the "scheduler does not yet consume delta
signatures end to end" bullet: narrow its last clause. Today it reads that a **single-component**
key-addressed edge dispatches and "the residue is a downstream that ALSO has an inbound edge or
source the key-addressed cell does not cover — that composed multi-component case still keeps the
ordinary route". After this phase: several key-addressed edges into one downstream compose and
each dispatches; the residue is only an inbound input that is **not** key-addressed (a declared
source, or a model edge that resolved no cell), which widens to the ordinary route with a
reported downgrade. No other spec text changes — §"Dispatch" already pins this behaviour.

## Tests

Red-green, all in `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` unless noted.

1. `resolve_key_addressed_cells_returns_one_cell_per_keyed_edge` (unit) — the new plural resolver
   returns a cell for **each** keyed edge of a two-upstream model, not just the first.
2. `resolve_key_addressed_cells_fails_loud_when_one_edge_key_is_missing` (unit) — with two keyed
   edges where one's `key_scope` names a column that upstream does not carry, the resolver still
   refuses by name (`MaintenanceRepairKeysNotDiscoverable`/`...ColumnMissing` as today) rather
   than silently returning only the healthy cell.
3. `two_keyed_upstreams_dispatch_both_cells` (e2e, DuckDB) — a `grain: partition` downstream
   reading two clockless `keyed upsert` upstreams; one key changes in *each* upstream in the same
   tick; both repairs land, the result equals a full-refresh oracle, and a key touched by neither
   upstream is bit-identical (no whole-table rewrite).
4. `uncovered_input_widens_and_reports_the_downgrade` (e2e) — the existing uncovered-`sources.flags`
   fixture, now run with a recording reporter: the ordinary route is still taken, the result is
   still correct, and exactly one `dispatch_widened` advisory fires naming the uncovered input.
5. `full_coverage_reports_no_downgrade` (e2e) — the fully-covered fixture from test 3 fires no
   `dispatch_widened` advisory.
6. `single_edge_dispatch_is_unchanged` — phase 2's
   `partition_grain_downstream_dispatches_key_addressed_cell` and
   `keyed_chain_maintains_only_the_changed_keys_end_to_end` must pass untouched (regression pin;
   no new test body, assert by running them).

## Tasks

1. `crates/smelt-runtime/src/maintenance_driver.rs`: add
   `resolve_live_key_addressed_model_edge_cells(...) -> Result<Vec<LiveKeyAddressedModelEdgeCell>>`
   collecting **every** `Technique::PerGroupRecompute` cell that carries a `key_scope` (plan
   derived exactly once — maintenance-plan purity). Keep every existing fail-loud leg, now per
   cell. Re-express `resolve_live_key_addressed_model_edge_cell` as a delegating wrapper over it
   (first cell) so existing callers/tests are unchanged.
2. `crates/smelt-runtime/src/execute.rs`: resolve `key_addressed_edge_cells: Vec<_>` once, above
   the `plan_is_keyed` gate (replacing the singular binding).
3. Replace the non-keyed substitution gate (`execute.rs` ~2703) with a coverage gate:
   no declared-source ref, and every entry of `keyed_model_edges` resolved a cell
   (`key_scope.from` set covers them), and `plan.model_file.refs.len() == keyed_model_edges.len()`.
   `N >= 1` is now admitted where only `N == 1` was.
4. Dispatch **each** resolved cell in sequence at that site: sum `row_count` into `total_rows`;
   the manifest `strategy` label is `diff_patch` when any cell resolved a `DiffPatch` write, else
   `per_group_recompute` (identical to today for the single-cell case); `batch_safety` stays
   `key_addressed`; one `model_completed` for the model, not one per cell.
5. Do the same fold at the keyed-branch dispatch site (`execute.rs` ~2047): iterate the resolved
   cells, summing rows, preserving `used_per_group_recompute`/`used_diff_patch` as "any cell".
6. `crates/smelt-runtime/src/reporter.rs`: add `fn dispatch_widened(&self, _run_id: &str,
   _model: &str, _reason: &str) {}` (default no-op) — the visible leg of §"Widen-never-narrow at
   dispatch". Call it from the non-keyed site when cells resolved but the coverage gate refuses,
   with a reason naming the uncovered inbound refs.
7. `crates/smelt-cli/src/reporter.rs` (`CliReporter`): implement `dispatch_widened` as a warning
   line naming the model and the uncovered inputs.
8. Narrow the Known Divergences bullet per §Spec delta above.

## Verification

- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test typed_edge_graph`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches (timeless-oracle rule)

## Commit message

`feat(incremental): compose key-addressed dispatch across every covered inbound edge`
