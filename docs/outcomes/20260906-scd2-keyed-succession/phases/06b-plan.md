# Phase 6b — Deferral frontier for the succession grain

## Objective

Make `contract.deferral`'s executed run-skip reachable for a succession model. Today the
succession dispatch in `crates/smelt-runtime/src/execute/project/mod.rs` returns
`ModelOutcome::Completed` before the ordinary path's interval-ledger and landed-delta writes,
so `contract_probes::resolve_deferral_frontiers` reads `(None, None)` and
`deferral::run_license` can only ever answer `Run`. Record both frontiers from the succession
window-forward path, then land phase 7d's tests 6–7 verbatim. Advances **criterion 6** (the
contract-lattice `deferral` leg's executed-skip clause, the residue 7d recorded).

## Spec delta

`docs/specs/incremental_shapes.md` §"The succession grain" → §"Run shape and late events":
add one sentence — a succession model's completed window-forward run records its run window
in the interval ledger and its driving source's landing on exactly the same terms as every
other maintained grain (`run_state.md` §"Interval ledger", §"Landed-delta"), which is what
makes `contract.deferral`'s frontier lag measurable for this grain; the whole-source rebuild
path (`--full-refresh` / `smelt rebuild`) has no run window and records neither. No change to
constraint 12, which already states `deferral` is admitted with unchanged semantics.

## Tests

Runtime (`crates/smelt-runtime/tests/`):

1. `succession_run_records_its_maintained_interval` (new `succession_frontiers.rs`) — RED
   today: after one window-forward `execute_project` over a succession model,
   `.smelt/` `intervals.json` carries `[start, end)` for that model under its own
   `compute_model_hash` key.
2. `succession_run_records_its_source_landing` (same file) — after the same run,
   `landed_deltas.json` carries the driving source's `[start, end)` with `AppendOnly`
   posture; `resolve_deferral_frontiers`-style read-back yields `Some` for both frontiers.
3. `succession_rebuild_records_no_frontier` (same file) — a `--full-refresh` / `rebuild: true`
   succession run leaves both stores untouched (no run window exists to record).
4. `succession_deferral_skip_is_licensed_end_to_end`
   (`crates/smelt-runtime/tests/contract_deferral_skip_e2e.rs`, or a sibling file if that one
   is at its large-file cap) — the three-run A/B/C shape of the existing ordinary-grain e2e
   over two succession models, asserting run C's manifest entry is
   `strategy == "skipped_deferral"`, `RunOutcomeKind::Skipped`, `row_count == 0`, and that
   neither the presented table nor the tombstone ledger changed across run C.

Conformance (`crates/smelt-cli/tests/maintenance_conformance/contract_points.rs`) — re-add
phase 7d's tests 6–7 as written in `phases/07d-plan.md`, unweakened:

5. `succession_deferral_recipe_upholds_restated_oracle_with_a_skipped_run`.
6. `succession_deferral_leg_is_not_vacuous` — METAMORPHIC: the same post-skip state FAILS
   `ContractPoint::Default`.

## Tasks

1. Add `crates/smelt-runtime/src/maintenance_driver/succession/frontier.rs`: one
   `pub(crate) async fn record_succession_frontiers(file_store, state_io_lock, model_name,
   model_hash, source_facts, start_str, end_str)` that takes the `state_io_lock` critical
   section and mirrors the ordinary path's two blocks (`mod.rs` ~3974–4052) exactly —
   `interval_store.get_or_create(...).record_interval(...)` then per-source `record_landing`
   with the same `partition_col`/`mutation` → `SourceMutationPosture` mapping. No new
   posture logic: lift, do not re-derive. Unit-test the posture mapping in place.
2. Call it from the succession dispatch's **window-forward** branch only, after
   `execute_succession_maintenance` returns `Ok` (all steps folded) and before the
   `manifest_entries.insert`, passing the branch's own `s`/`e` formatted `%Y-%m-%d` and the
   already-built `succession_source_facts`. The rebuild branch does not call it.
3. Keep `mod.rs` at or below its large-file baseline (4689 — it is *at* the cap, so any net
   growth fails `large-file-check.sh`). Pay for the new call by moving the succession
   `ModelRunRecord` literal (~20 lines) into the same new `frontier.rs`/`succession` module
   as a small constructor; verify with `bash .claude/scripts/large-file-check.sh`.
4. Add the four runtime tests (1–4); confirm 1, 2 and 4 are red before task 1/2 land.
5. Re-add conformance tests 5–6 into `contract_points.rs` verbatim from `phases/07d-plan.md`,
   using the existing `assert_succession_equivalence_at_point(project, recipe, point,
   processed_arrival_frontier, input_frontier)` — it is already frontier-aware, so no new
   comparator. The `contract:` field on `SuccessionRecipe` and its frontmatter rendering
   already exist (phase 7d).
6. Apply the spec delta above.
7. Note in the summary whether per-cell `contract.cells[].deferral` frontier advancement
   (`advance_cell_frontiers`) is meaningful for a grain with exactly one derived cell — this
   phase deliberately wires the model-level frontier only.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test succession_frontiers --test contract_deferral_skip_e2e`
- `cargo test -p smelt-runtime --test succession_patch_e2e --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` (full seeded sample)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(succession): record run-state frontiers so contract.deferral can license a skip`
