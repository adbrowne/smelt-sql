# Phase 23 summary — `--select` scoping for `--since-upstream`

## Shipped

- `smelt_runtime::propagation::scope_plan_to_selection` (`crates/smelt-runtime/src/propagation.rs`)
  — pure function narrowing a `SinceUpstreamPlan` to a selected model set, given the direct
  (one-hop) model-upstream map. Retains selected runs, appends `SUPPRESSED (not selected)` lines
  to the report for deselected dirty models, and refuses (`bail!`, naming both models) when a
  retained run's direct upstream is itself dirty but was dropped by the selector.
- `crates/smelt-cli/src/commands/run.rs::run_since_upstream` now resolves `--select`/`--exclude`
  through the same `compute_scope` + `resolve_selector_args` + `select_executable_models` pass
  the ordinary run path uses, then calls `scope_plan_to_selection` right after the propagation
  plan is computed. Emits `smelt: no models matched the selector(s)` (exit 0) when scoping empties
  a non-empty run set, distinct from the pre-existing "propagated nothing" message for a plan that
  was already empty before scoping.
- Spec: `docs/specs/incremental_models.md` §CLI's `--since-upstream` bullet documents selector
  scoping, the refusal posture, and the no-op contract; §Known Divergences' "Graph-layer gaps"
  bullet no longer names "no `--select` scoping exists".
- Docs-site: `docs-site/docs/reference/cli.md` §"Forward propagation with `--since-upstream`" gets
  a paragraph + `--select +marts.revenue` example.
- Tests: 5 pure unit tests in `crates/smelt-runtime/tests/since_upstream_propagation.rs`
  (`scoping_*`), 4 end-to-end CLI tests in `crates/smelt-cli/tests/since_upstream.rs`
  (`select_*`), all matching the plan's test list exactly.

## Decisions

- Selection is computed via `smelt_runtime::select::select_executable_models` (the same
  test-model/generator-file filtering the ordinary run path gets) rather than a bespoke
  model-name filter, so `--select`/`--exclude` behave identically across both run paths.
- The upstream map passed to `scope_plan_to_selection` is direct (one-hop) dependencies only
  (`DependencyGraph::get_upstream`), not transitive — the refusal check only needs to catch a
  *direct* dirty-but-deselected upstream; a transitively-dirty-but-not-directly-connected model
  can't stale the retained run through an edge that doesn't exist.

## For the next planner

- No follow-up work surfaced beyond the outcome's existing phase 24 (`examples/web_analytics`
  full `--since-upstream` compatibility), which is unaffected by this phase.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test since_upstream_propagation` — 29 passed.
- `cargo test -p smelt-cli --features duckdb --test since_upstream` — 17 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1 ignored.
- `.claude/hardening-baseline.txt` updated (`smelt-cli println` 171→172: one new
  `eprintln!("smelt: no models matched the selector(s)")`, same pattern as the existing
  ordinary-run-path message).
