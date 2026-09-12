# Phase 09 summary — Composed-upstream granularity closed

## Shipped

- `ClampAndLocality` (`crates/smelt-runtime/src/propagation/clamp_locality.rs`) now returns its
  already-computed `composed_sources` fixed point instead of discarding it.
- `crate::propagation::composed_source_granularities` (`propagation/mod.rs`) — a `pub(crate)`
  accessor exposing that map.
- `execute_project` (`execute/project/mod.rs`) computes the map once, gated on at least one
  referenced source declaring `retention:`, and threads it through to
  `derive_model_retention_plan`.
- `derive_model_retention_plan` (`execute/retention_admission.rs`) now extends its
  `SourceFacts`/clocked-granularity candidate pools for `grain: key` models with every referenced
  address the composed map resolves — mirroring `smelt-db`'s `maintenance_refs/plan.rs`.
- Two counterexample-pinning tests in `smelt-logical`'s `locality.rs`
  (`adding_a_candidate_to_an_empty_pool_resolves_an_undecided_granularity`,
  `adding_a_candidate_to_an_already_ambiguous_pool_stays_undecided`).
- Two new tests in `retention_admission.rs`
  (`a_keyed_model_over_a_composed_upstream_resolves_locality_via_composed_sources`,
  `a_composed_upstream_contributes_no_retained_bound`), plus the two pre-existing
  `derive_model_retention_plan` call sites updated for the new parameter.

## Decisions

- Planning's leg-3 argument ("adding a candidate can only take `Some → None`") is FALSE for the
  empty-pool case — pinned with a counterexample rather than the false universal, then closed via
  task 6 rather than recorded as proven. See outcome.md Decision log 2026-09-09 (implementation)
  for the full argument, including why the divergence never actually moved a real
  `retention_reaches`/`retention_downgrades` value (a `retention:` source is always its own
  declared clocked candidate) — the fix is still correct to make because it removes a spurious
  `Refusal::LocalityNotEstablished` this call site's plan carried, latent today but exactly the
  class of diagnostics-vs-runtime divergence this outcome exists to close.
- Left leg 4 (diagnostics-severity gating in `smelt-db`) untouched — out of `smelt-runtime`'s
  scope and already covered by earlier phases; did not add a duplicate test for it.

## For the next planner

- No spec delta: this was a derivation-internal divergence with no user-visible surface change,
  confirming the plan's own prediction.
- Rows 10 (`smelt explain`/docs) and 11 (close-out) are unaffected in scope — `smelt explain
  --json` already reads the diagnostics path, which was always correct; only the run-time
  re-check path needed the fold.
- Nothing deferred out of this phase's scope.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  test, example_diagnostics). Required updating `.claude/large-file-baseline.txt` via
  `--update` for the two files this phase legitimately grew
  (`smelt-logical/src/maintenance/locality.rs` 2089→2124,
  `smelt-runtime/src/execute/project/mod.rs` 4874→4898).
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test availability_seam` — PASS (6+4+41).
- `cargo test -p smelt-logical --test walk_coverage` — PASS (14).
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (104).
- `bash .claude/scripts/large-file-check.sh` — PASS (after baseline update above).
