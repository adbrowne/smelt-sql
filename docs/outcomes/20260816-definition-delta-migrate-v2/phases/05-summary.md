# Phase 5 summary — generative definition-edit schedules

## Shipped

- `crates/smelt-maintenance-testkit/src/schedule_gen.rs`: `arb_schedule_with_definition_edit(recipe)`
  — builds the same base window/late-row schedule `arb_schedule_for` does, then splices a
  `RewriteModel { edit }` + `FullRefreshRun` pair at a generated index in `[1, n_windows]` (always
  after the first `RunWindow`, before the trailing late-row/catch-up steps). `edit` is drawn from
  `recipe.evolution`; empty evolution yields the plain schedule unchanged.
- `is_permutable` now excludes any schedule containing a `RewriteModel` step.
- `ConformanceStep::RewriteModel`'s doc comment refreshed — the stale "classification is unbuilt"
  paragraph replaced with a pointer to phase 6 (the migrate-gated recovery leg).
- Three new `schedule_gen.rs` unit tests: rewrite-then-recovery shape, non-permutability, and
  "drawn edit is always in `recipe.evolution`".
- Two new standing gate tests in `crates/smelt-cli/tests/maintenance_conformance/gate.rs`:
  `definition_edit_pool_upholds_equivalence` (generic leg, `case_count()` cases, asserts
  `rewritten_cases > 0`) and `definition_edit_grouping_column_upholds_equivalence` (the
  `AddGroupingColumn` skeleton-widening leg, pinned rather than left to the draw, over the four
  aggregate constructs).
- `docs/specs/definition_deltas.md` §Known Divergences: removed "The conformance harness has no
  definition-edit step kind yet"; fixed the diagnostic-rename bullet's stale `phase 6` → `phase 8`
  pointer.

## Decisions

- Confirmed the phase-5 planning reshape (see outcome.md decision log): a NEW sibling generator,
  not a widening of `arb_schedule_for` itself, so the six other consumers of that generator stay
  unaffected.
- For the pinned `AddGroupingColumn` test, spliced manually at index 1 (right after the first
  `RunWindow`) rather than reusing `arb_schedule_with_definition_edit`'s random `edit_idx` draw —
  the aggregate constructs' evolution also contains `AddPayloadColumn`, so leaving it to the draw
  would only probabilistically exercise the skeleton-widening leg.

## For the next planner

- Phase 6 (migrate-driven recovery leg) is next: only the full-refresh arm of `execute.rs` calls
  `save_deployed_schema`, so an incrementally-maintained model has no recorded definition
  mid-history and `smelt migrate` fails closed with `NoRecordedDefinition`. That gap blocks driving
  `smelt migrate --apply` as the recovery step in a generative schedule.
  No other follow-up work surfaced — the AddGroupingColumn leg admitted and passed cleanly, no
  diagnosis/narrowing was needed.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-maintenance-testkit --quiet` — 34 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet` — 81 passed.
- `SMELT_CONFORMANCE_CASES=40 cargo test -p smelt-cli --test maintenance_conformance --features duckdb definition_edit` — 2 passed.
- `cargo check -p smelt-cli --tests --features spark` — clean (Spark mirror still compiles).
