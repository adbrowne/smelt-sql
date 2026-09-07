# Phase 3a summary — the succession diagnostics surface

## Shipped

- Eleven `Succession*` `DiagnosticCode` variants in
  `crates/smelt-db/src/diagnostics_types.rs`, grouped under one doc block.
- `smelt_logical::maintenance::refusal_code` now maps `Refusal::SuccessionNotRecognized`
  exhaustively over `NotSuccessionReason`'s ten variants (was a blanket `None`).
- `SuccessionDerivation` (`crates/smelt-logical/src/maintenance/succession.rs`) carries
  `advisories: Vec<SuccessionAdvisory>` alongside `plan`/`output` — a structural proof
  the advisory cannot affect admission, backed by `advisory_does_not_change_the_derived_plan`.
- `MaintenanceRefusal::SuccessionNotRecognized` (`refusal_diag.rs`) and its
  `diagnostic_for_refusal` mapping to the ten Error codes; `queries/maintenance/diagnostics.rs`'s
  `Refusal` → `MaintenanceRefusal` projection now maps it instead of dropping to `None`.
- `MaintenancePlanResult`/`MaintenancePlanDiagnostics` gained `succession_advisories`,
  threaded from `derive_succession_plan` through to `file_check.rs`, which folds it into a
  `Warning` `SuccessionPreFilterNegatesFlag` diagnostic.
- `examples/broken/models/sources/succession_changes.yml` + eleven
  `examples/broken/models/succession_*.sql` fixtures, one per code.
- `smelt-cli`'s `broken_workspace_succession_codes` test: each fixture fires exactly its
  own code at its own file, the advisory fixture is `Warning` with no `Succession*` Error.
- `smelt-lsp/src/backend.rs`'s `diagnostic_code_str` extended with the eleven kebab-case
  wire strings (LSP/CLI parity, `example_workspaces` green).

## Decisions

- **Fixed a real Salsa-wiring bug, not just diagnostics plumbing**: `smelt-db`'s
  `#[salsa::tracked] maintenance_plan` (`src/maintenance_refs/plan.rs`) early-returned the
  empty default whenever `resolved_grain().is_none()` — a guard written before the
  succession grain existed, which silently discarded every succession diagnostic before
  it could ever reach `check_file_diagnostics`. Removed the `resolved_grain.is_none()` half
  of the guard; the plain `refresh != Incremental` half is unchanged. Left the sibling
  `maintenance_plan_report` (used by `smelt explain`) with the same stale guard —
  out of this phase's scope (explain is phase 8), noted below.
- Reused `examples/broken/models/sources/maintenance_orders.yml` (mutation_profile:
  append_only, no `timeseries:`) directly as the sole driving source for the
  `SuccessionDrivingSourceNotAppendOnly` fixture, and as a JOIN partner for
  `SuccessionSingleSourceOnly` — matches the plan's own suggestion, no new source needed
  beyond the one `succession_changes.yml` this phase adds.
- `SuccessionOrderNotMonotoneClock`'s fixture orders by `customer_id` (not a real clock)
  while also projecting `LEAD(customer_id)` over it — `record_window`'s own reach-check
  requires the `LEAD`/`LAG` argument to equal the `ORDER BY` column, so a mismatched
  argument earlier trips `SuccessionWindowFunctionNotLead` instead; matching both to the
  same (non-clock) column is what isolates the intended refusal.

## For the next planner

- **`maintenance_plan_report`** (`smelt-db/src/maintenance_refs/plan.rs`, used by
  `smelt explain`) still carries the pre-succession `resolved_grain.is_none()` early-return
  guard — `smelt explain` on a succession model currently reports no plan at all. Phase 8
  ("Explain surface") must fix this guard the same way this phase fixed `maintenance_plan`,
  or explain will silently show nothing for the grain it's meant to render.
- **Large-file ratchet regression** (my own diff, same shape as phases 2b/3):
  `crates/smelt-db/src/queries/maintenance/{diagnostics,mod,plan,refusal_diag,tests}.rs`,
  `crates/smelt-db/tests/integration/refusal_codes/{fixtures,tests}.rs`,
  `crates/smelt-logical/src/maintenance/{refusal,succession}.rs`, and
  `crates/smelt-lsp/src/backend.rs` each grew a few lines past their baseline. Not fixed
  here per `docs/outcome_loop.md` §"The large-file shrink step" (dedicated non-blocking
  loop step); `backend.rs` in particular (5994 lines) is a long-standing shrink candidate.
- **Four gates were already red before this phase started** (confirmed by stashing this
  phase's diff and re-running each in isolation on the committed tree) — none are this
  phase's target, none touched by this diff:
  - `smelt-cli --test state_docs_freshness::spec_references_are_live` — `docs/specs/state.md`
    §References cites `crates/smelt-logical/src/maintenance/availability.rs`, which no
    longer exists after the `availability.rs` → `availability/{mod,state_structure}.rs`
    split (`c89eda1f`). Needs a doc-reference fix, not a code fix.
  - `smelt-core --test hardening_budget::gate_detects_regression` — `smelt-logical`
    `.expect(` count reads 14 against a baseline of 1. Not reproduced by my diff (confirmed
    on the stashed clean tree); likely another file-split artifact tricking the
    `#[cfg(test)]`-boundary scan, same class of bug as `join_context_reach`'s.
  - `smelt-logical --test contract_lattice_spec` — two failures, one citing a missing
    `crates/smelt-logical/src/contract/frozen_horizon.rs` (again looks like a split-vs-spec-
    reference drift).
  - `smelt-logical --test walk_coverage::admission_paths_have_no_raw_text_scans` — flags
    ten `.contains(...)` calls in `maintenance/choice/{keyless_write_suppression_tests,
    tests,write_suppression_tests,write_variant_tests}.rs` as unclassified raw text-scans;
    these are plain test assertions, not admission-path scans — looks like another
    file-split test-boundary miss (same shape as `join_context_reach`'s own bug, which this
    outcome's phase 3b is already scoped to fix for its own gate).
  All four look like fallout from this branch's large-file-splitting work landing ahead of
  this phase and are worth a dedicated hygiene pass — likely the same fix shape as phase 3b's
  (`join_context_reach`'s file-level `#[cfg(test)]` exclusion gap), possibly worth widening
  3b's scope or adding a sibling phase before phase 10's "all standing gates green" claim.
- `join_context_reach::every_production_join_context_new_is_tagged` remains red exactly as
  the phase-3 summary and this outcome's decision log describe — phase 3b's target, untouched
  here.

## Gates

- `bash .claude/scripts/verify-phase.sh` — fmt PASS, clippy PASS (both feature sets),
  `example_diagnostics` PASS. The bundled `cargo test (workspace)` leg is red, but only on
  the pre-existing/out-of-scope failures listed above plus the large-file ratchet — every
  succession-related test is green (see below).
- `cargo test -p smelt-db --test integration refusal_codes diagnostics_catalogue` — PASS
- `cargo test -p smelt-logical --lib refusal_code_tests --lib maintenance::succession` — PASS
- `cargo test -p smelt-cli --test example_diagnostics` — PASS (123 passed, incl. the new
  `broken_workspace_succession_codes`)
- `cargo test -p smelt-lsp --test example_workspaces` — PASS (35 passed)
- `cargo fmt --all -- --check` — PASS
