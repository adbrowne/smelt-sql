# Phase 23 plan — `--select` scoping for `--since-upstream`

## Objective

`smelt run --since-upstream` today ignores `--select`/`--exclude` entirely: the propagated plan
runs every dirtied model in the workspace. This phase makes the propagated plan **intersect**
with the selector — propagation still walks the whole graph (dirt must compose through
unselected intermediates), but only the selected dirty models execute, and a retained model
whose *dirty* upstream the selector dropped is refused fail-loud rather than run against a stale
input. Advances the success criterion that the graph-layer CLI surface has no unimplemented
gaps (`incremental_models.md` §Known Divergences' "no `--select` scoping exists" clause).

## Spec delta (make first)

- `docs/specs/incremental_models.md` §CLI, the `smelt run --since-upstream` bullet: add a
  sentence — the invocation accepts the ordinary `--select`/`--exclude` selectors
  (`model_selection.md` grammar, `+` graph operators included). Propagation is always
  whole-workspace; the selector narrows only which propagated runs execute. The printed dirty
  set still shows the whole propagated set, with the deselected lines marked suppressed. A
  retained dirty model whose direct upstream is itself dirty but deselected is refused with a
  diagnostic naming both (same posture as `cli.md` §"`--exclude` and working-set consistency");
  the user adds `+<model>` or drops the downstream. An intersection that is empty is a quiet
  no-op (`smelt: no models matched the selector(s)`, exit `0`, per `cli.md`).
- Same file §Known Divergences, "Graph-layer gaps" bullet: delete the trailing
  "; no `--select` scoping exists".
- `docs-site/docs/reference/cli.md` §"Forward propagation with `--since-upstream`": one short
  paragraph on selector scoping + the dirty-upstream refusal, with a `--select +marts.x` example.

## Tests (red first)

Runtime (pure), `crates/smelt-runtime/tests/since_upstream_propagation.rs`:
1. `scoping_keeps_only_the_selected_runs` — plan with runs A,B,C; selection {B} ⇒ runs == [B]
   (given A is not dirty).
2. `scoping_suppressed_runs_are_still_reported` — the returned report contains a
   `SUPPRESSED (not selected)` line naming each dropped dirty model.
3. `scoping_refuses_a_dirty_upstream_dropped_by_the_selector` — A and B both dirty, selection
   {B}, edge A→B ⇒ `Err` whose message names `B` and `A`.
4. `scoping_admits_a_clean_upstream_outside_the_selection` — A not dirty, B dirty, selection
   {B} ⇒ `Ok`, runs == [B].
5. `scoping_with_an_empty_selection_yields_no_runs` — selection {} ⇒ `Ok` with empty `runs`.

CLI (end to end), `crates/smelt-cli/tests/since_upstream.rs` (reuse `stage_model_chain`):
6. `select_narrows_the_since_upstream_run_set` — `--select` on the downstream only (with its
   upstream not dirty) executes just that model; the other stays untouched.
7. `select_with_upstream_operator_keeps_the_dirty_chain` — `--select +<downstream>` runs both.
8. `select_dropping_a_dirty_upstream_refuses` — non-zero exit, stderr names both models.
9. `select_matching_nothing_is_a_quiet_no_op` — exit `0`, stderr `no models matched the
   selector(s)`, nothing executed.

## Tasks

1. Land the spec delta + docs-site paragraph.
2. Add `pub fn scope_plan_to_selection(plan: &SinceUpstreamPlan, selected: &BTreeSet<String>,
   upstreams: &BTreeMap<String, BTreeSet<String>>) -> Result<SinceUpstreamPlan>` to
   `crates/smelt-runtime/src/propagation.rs` — pure, takes the direct-upstream map so it does
   not depend on `DependencyGraph`; filters `runs`, appends the suppressed footer to
   `dirty_set_report`, and refuses on a dirty-but-deselected direct upstream of a retained run.
3. In `crates/smelt-cli/src/commands/run.rs::run_since_upstream`: build the selector-resolution
   salsa db (`compute_scope` + `resolve_selector_args`, as the ordinary path does — move the
   `since_upstream` early return below that resolution or replicate it inside the function),
   expand to a model set via `graph.select_models` / `graph.exclude_models`, and call
   `scope_plan_to_selection` right after `plan_since_upstream_with_observed_deltas`. When
   `--select`/`--exclude` are both empty, skip the call entirely (unchanged behaviour).
4. Emit the `smelt: no models matched the selector(s)` stderr line + exit `0` when scoping
   empties the run set (distinguish from the existing "propagated nothing" message).
5. `run_since_upstream` must still pass the single propagated model in each `ExecuteRequest.select`
   — the scoping happens on the plan, not by forwarding selectors into `execute_project`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test since_upstream_propagation`
- `cargo test -p smelt-cli --features duckdb --test since_upstream`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`feat(propagation): intersect the --since-upstream propagated plan with --select/--exclude`
