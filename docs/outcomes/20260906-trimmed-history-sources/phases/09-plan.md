# Phase 09 plan — Composed-upstream granularity: close the silent skip or prove it unreachable

## Objective

Settle the residual divergence phases 7 and 8 handed over: `derive_model_retention_plan`
(`crates/smelt-runtime/src/execute/retention_admission.rs`) resolves
`driving_source_granularity` over declared clocked source refs alone, while the diagnostics
path (`crates/smelt-db/src/maintenance_refs/plan.rs`) additionally folds in composed-upstream
model candidates. Either that divergence can suppress a retention verdict for a model that
still runs — in which case it is closed — or it provably cannot, in which case the argument is
recorded as executable tests plus a doc comment on the derivation. Advances criteria 4 (never
silent) and 5 (the bound moving is an event).

## The reachability argument to test (planning found it, the phase must verify it)

Planning read the code and found the divergence runs in the *permissive* direction, not the
silent one. Four legs, each of which the tests below pin; **if any leg fails, the phase takes
the closure branch in task 6 instead**:

1. `retention:` without `timeseries:` is refused (`crates/smelt-core/src/sources.rs`), so every
   retained ref is necessarily a *clocked* ref.
2. `build_succession_source_refs` and `model_source_retentions` filter `model_file.refs` against
   `source_infos` identically, so a retained ref is always in the run-time clocked pool.
3. The run-time pool is a subset of the diagnostics pool (same declared refs; diagnostics adds
   composed-upstream candidates), and `single_clocked_granularity` is "exactly one element else
   `None`" with no dedup — so adding candidates can only take `Some → None`, never `None → Some`.
   A run-time `None` therefore implies a diagnostics `None`, i.e. `Refusal::LocalityNotEstablished`.
4. That refusal is `DiagnosticSeverity::Error` (`queries/maintenance/refusal_diag.rs`), folded
   into `file_check` and blocked pre-execution by `crate::gate::gate_diagnostics`
   (`execute/project/mod.rs:190`) — the model never runs, so the skipped fold is never silent.

## Spec delta

None expected: this is a derivation-internal question with no user-visible surface change. If
task 6's closure branch is taken and it makes a previously-running model shape refuse,
`docs/specs/sources.md` §Semantics 5 "Retention refusal" gains one sentence naming the shape,
edited before the code.

## Tests (red-green; for a leg that is already green, demonstrate sensitivity by temporarily
breaking its premise and showing the test fails)

1. `retention_without_a_clock_is_refused` — a source declaring `retention:` and no
   `timeseries:` is refused with its named code (leg 1; extend the existing `sources.rs`
   validation coverage rather than duplicating it if already pinned).
2. `adding_a_candidate_never_resolves_an_undecided_granularity` — unit over
   `single_clocked_granularity`: for every pool P and superset P′, `P → None` implies
   `P′ → None` (leg 3).
3. `a_keyed_model_with_two_clocked_sources_is_diagnostics_refused_before_it_runs` — stage a
   `grain: key` model referencing two clocked sources (one `retention:`-bearing); assert the
   derivation returns `locality_refused_plan` (empty `retention_reaches`) AND that the same
   workspace produces an Error-severity `KeyedForbidsTimeseries` diagnostic, so the run is
   gated (leg 4).
4. `a_keyed_model_over_a_composed_upstream_never_runs_with_an_empty_retention_fold` — stage a
   `grain: key` model referencing both an upstream maintained model and a retained clocked
   source; assert the disjunction row 9 is about: either `retention_reaches` is non-empty and
   `check_retention_admission` refuses at an aged window, or the model is diagnostics-refused.
   Never "runs with empty `retention_reaches`".
5. `a_composed_upstream_contributes_no_retained_bound` — `model_source_retentions` over a model
   whose refs include an upstream *model* returns entries for declared sources only:
   `retention:` is a source-only declaration, so a composed candidate can never carry a bound
   the run-time pool would be missing.

## Tasks

1. Confirm leg 1 is pinned; add test 1 if the existing coverage does not assert the named code.
2. Add test 2 next to `single_clocked_granularity` (`crates/smelt-logical/src/maintenance/locality.rs`).
3. Add tests 3-5 to `retention_admission.rs`'s test module, reusing its existing
   `stage_source`/`discover` helpers (real staged files, no backend).
4. Run every test; for each that is green on first write, break its premise once to prove it is
   a real assertion, and record that in the summary.
5. If all four legs hold: replace the stale "tracked for phase 8" note in
   `derive_model_retention_plan`'s doc comment with the four-leg argument above, naming the
   tests that pin each leg — the divergence is deliberate and bounded, not an outstanding gap.
6. If any leg fails: close it instead — return the converged `composed_sources` map from
   `derive_clamp_and_locality` (add it to `ClampAndLocality`, expose a `pub(crate)` accessor in
   `crate::propagation`), compute it ONCE in `execute_project` before the model loop (gated on
   at least one referenced source declaring `retention:`, so existing projects pay nothing),
   thread it into `derive_model_retention_plan`, and extend the `SourceFacts`/clocked-granularity
   pools for `grain: key` models exactly the way `maintenance_refs/plan.rs` does. Never a second
   independent derivation of composed-source admission.
7. Record the verdict (proved unreachable, or closed) as a dated Decision log line in
   `outcome.md`, and note in `phases/09-summary.md` whether row 10/11 scope is affected.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test availability_seam`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(maintenance): settle the composed-upstream granularity divergence at the retention call site`
