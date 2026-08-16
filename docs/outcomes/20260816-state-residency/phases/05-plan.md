# Phase 5 plan — two-step ideal-then-availability derivation

## Objective

Land the degradation contract's second step (`state.md` §"The degradation contract") as a pure
resolution pass over the already-derived ideal plan: a cell whose technique needs a state
structure the target backend cannot build is downgraded to the cheapest equivalence-preserving
recompute-family member, and the downgrade is recorded as an advisory
`MaintenanceStateDowngraded` visible in diagnostics and `smelt explain`. Advances success
criterion 3 (the downgrade half; `DeclaredContractRequiresState` is now row 6) and retires the
`tracing::warn!` phase 4 left at the frontier-record skip site.

## Spec delta

`docs/specs/diagnostics.md` §"Maintenance" catalogue table (near line 494): add one row —
`` `MaintenanceStateDowngraded` | Advisory | A cell's derived technique requires a state
structure with no realisation on the target backend; the cell was downgraded to its
recompute-family equivalent, naming the cell, the ideal technique, and the missing structure.
(`state.md` §"The degradation contract"). `` The `DiagnosticCode` variant this phase adds needs
the row for the catalogue coverage gate (`smelt-db/tests/integration/diagnostics_catalogue.rs`).
`state.md` §Surface already specifies both codes (phase 1) — no edit there.

## Tests

`crates/smelt-logical/tests/state_availability.rs` (new):
- `keyed_fold_without_ledger_downgrades_to_per_group_recompute` — a `Technique::KeyedFold` cell
  resolved against `StateAvailability` with no reconciliation ledger becomes
  `Technique::PerGroupRecompute`, carrying the fallback's own key scope/slice.
- `keyed_fold_without_ledger_and_no_fallback_refuses_loudly` — when per-group recompute was not
  admissible at derivation, resolution drops the cell and pushes a
  `Refusal::NoAdmissibleTechnique` whose `why` names the missing structure — never a silent
  keyed fold on a ledger-less backend.
- `region_recompute_without_frontier_record_records_a_downgrade` — a `DeleteInsert` cell keeps
  its technique (already recompute-family, equivalence unaffected) but records a downgrade
  naming the frontier record as the lost bookkeeping.
- `full_availability_is_a_no_op` — with every structure available the resolved plan equals the
  ideal plan cell-for-cell and records zero downgrades.
- `ideal_plan_survives_resolution` — the ideal plan object is still readable after resolution
  (no early pruning; the counterfactual `smelt explain` prints comes from it).

`crates/smelt-db/tests/maintenance_diagnostics.rs`:
- `state_downgrade_surfaces_as_an_advisory_diagnostic` — a keyed-fold model against a
  `spark` target yields a warning-severity `MaintenanceStateDowngraded` naming cell, ideal
  technique, and missing structure; against `duckdb` it yields none.

`crates/smelt-cli/tests/explain_maintenance.rs`:
- `explain_prints_a_downgraded_cell_with_both_techniques` — the cell block prints the executed
  technique *and* the technique that would run with the structure available.

## Tasks

1. Spec delta above (diagnostics.md catalogue row) — first, before any code.
2. New pure module `crates/smelt-logical/src/maintenance/availability.rs`:
   `StateStructure { ReconciliationLedger, FrontierRecord }`; `StateAvailability` (per-structure
   bools) with `all()` (the "assume ideal" value for a caller that does not know the target) and
   `none()`; `StateDowngrade { cell_group, trigger, ideal_technique, resolved_technique,
   missing_structure, why }`; `required_state_structure(Technique) -> Option<StateStructure>`
   (`KeyedFold` → ledger; `DeleteInsert` → frontier record as *bookkeeping*, never a correctness
   premise — document that distinction); and
   `resolve_state_availability(&MaintenancePlan, &StateAvailability) -> ResolvedPlan { plan,
   downgrades }` leaving its input untouched.
3. `PlanCell` gains `recompute_fallback: Option<RecomputeFallback>` (technique + the `scans` /
   `key_scope` the fallback's emitter needs), populated at the `Technique::KeyedFold` push site
   in `derive.rs` (~line 1330) by calling the existing `repair::admit_per_group_recompute` with
   the same inputs already in scope; `None` records nothing and drives test 2's refusal.
   Additive-field precedent: `PlanCell::key_scope`'s doc comment.
4. `smelt-db`: `state_availability_for(backend_name)` mirroring `backend_write_capabilities_for`
   (`queries/maintenance.rs` ~line 890) — `duckdb` has both structures, every other name has
   neither (the single owner of the name→struct mapping stays `smelt_dialect`; this only narrows
   what state builders `smelt-state` actually ships).
5. Thread availability through `derive_model_maintenance_plan` / `…_with_edges` as a new
   parameter and resolve **inside** them, so every consumer (runtime lowering included) reads the
   resolved plan without a per-call-site change; `MaintenancePlanResult` gains `ideal_plan` and
   `state_downgrades`. Call sites that know the target pass the real availability; the rest pass
   `StateAvailability::all()` (identical behaviour to today — no silent regression).
6. `DiagnosticCode::MaintenanceStateDowngraded` + warning-severity mapping in `smelt-db`'s
   maintenance diagnostics fold (alongside the existing `Maintenance*` refusal mapping).
7. `smelt explain`: print each downgraded cell's executed technique and the ideal technique it
   would run with the structure available (`build_maintenance_plan_report`, `explain.rs` ~line
   316, beside the existing `technique:` line).
8. Retire the frontier-skip `tracing::warn!` in `smelt-runtime`'s per-batch region-recompute path
   (phase 4) in favour of the recorded downgrade; keep a `debug!` if the run path still needs it.
9. Update `.claude/hardening-baseline.txt` only via the gate's own `--update`, if the new code
   moves a count; note the reason in the commit body.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full).
- `cargo test -p smelt-logical --test state_availability --test maintenance_plan_admission --test
  maintenance_plan_conformance --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`
- `cargo test -p smelt-cli --test explain_maintenance --test maintenance_conformance`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test
  technique_lowering`

## Commit message

`feat(state): resolve maintenance-plan state availability with recorded MaintenanceStateDowngraded`
