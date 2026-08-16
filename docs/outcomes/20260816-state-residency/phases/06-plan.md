# Phase 6 plan — `DeclaredContractRequiresState`

## Objective

Land the fail-loud half of criterion 3: a `contract.deferral` declaration in a project whose
posture (or backend) cannot supply the frontier it is measured against is refused by name at
validation time, instead of silently degrading into an unmeasured promise. Closes the second
of `state.md` §Surface's two new diagnostic codes and the `state.md` §"Declarations stay
fail-loud" exception.

## Discovery that shapes this phase

`contract.deferral`'s lag is *not* measured from the engine-resident frontier phase 4 built.
`smelt_runtime::contract_probes::resolve_deferral_frontiers` compares the model's `IntervalStore`
frontier against the `LandedDeltaStore` input frontier — **both observability-class `.smelt/`
structures**, listed under `intervals` in `state.md` §"`state.mode` and what each posture
provides" and gated off by phase 2 under `stateless`. So today `examples/timeseries`'
`daily_event_counts_deferred` (project posture defaults to `stateless`) declares a deferral
window whose probe can never fire — exactly the "declared guarantee turned into an unverified
hope" the spec forbids. The refusal's real trigger is therefore the **posture**, with the
backend leg kept general in the validator's shape.

## Spec delta (first)

- `docs/specs/state.md` §"Declarations stay fail-loud": correct the one clause that says
  deferral's lag is measured from *the frontier* (which reads as correctness-class, hence
  posture-proof). It is measured from the run's interval/landed-delta frontier — an
  observability-class structure the `stateless` posture withholds — so
  `DeclaredContractRequiresState` fires when the posture, or a backend with no realisation of
  that structure, cannot supply it. One or two sentences; the §Diagnostics row already says
  "posture or backend" and needs no change.
- `docs/specs/diagnostics.md` §"Maintenance" (beside the `MaintenanceStateDowngraded` row added
  in phase 5): add the missing `DeclaredContractRequiresState` catalogue row — Error severity,
  naming the declaration and the missing structure. Without it the `smelt-db --test integration`
  catalogue-coverage gate fails on the new code.
- No `incremental_models.md` change: §"The contract lattice" already names
  `DeclaredContractRequiresState` as deferral's refusal.

## Tests (red → green)

New `crates/smelt-logical/tests/contract_state_requirements.rs`:
- `deferral_requires_the_interval_frontier` — `required_state_structures` maps a model-level
  `contract.deferral` to the interval-frontier structure, naming the declaration.
- `cell_deferral_requires_the_interval_frontier` — each `contract.cells[].deferral` yields its
  own requirement, named by its `on:` address.
- `frozen_horizon_requires_no_state` — the frozen-horizon point produces no requirement (it
  degrades with `ProbeBaselineUnavailable`; phase 1's decision).
- `available_structure_yields_no_refusal` — validating against a `StateAvailability` that has
  the structure returns an empty refusal set.
- `absent_structure_refuses_naming_declaration_and_structure` — the refusal carries both.

Extend `crates/smelt-db/tests/contract_deferral_diagnostics.rs`:
- `deferral_under_stateless_posture_is_refused` — Error `DeclaredContractRequiresState`.
- `deferral_under_intervals_posture_is_clean` — no new diagnostic.
- `model_narrowing_to_stateless_refuses_declared_deferral` — the effective (narrowest) posture
  is what the check reads, not the project's alone.
- `cell_deferral_under_stateless_posture_is_refused` — one refusal per offending cell.

New test in `crates/smelt-runtime/tests/` (alongside `contract_deferral_skip_e2e.rs`'s harness
shape): `stateless_project_declaring_deferral_refuses_the_run` — `execute_project` fails via
`gate_diagnostics` with the code named, proving the refusal actually blocks a build.

## Tasks

1. Make the two spec edits above (spec-first).
2. `smelt-logical` `maintenance::availability`: add the interval/landed-delta frontier to
   `StateStructure` (observability-class; keep `ReconciliationLedger`/`FrontierRecord` intact).
3. `smelt-logical` `contract/`: pure `required_state_structures(&ContractConfig)` +
   `validate_contract_state(&ContractConfig, &StateAvailability) -> Vec<ContractStateRefusal>`
   (declaration name, missing structure, why). Single owner — `smelt-db` decides nothing.
4. `smelt-db`: `state_availability_for_project(backend_name, state_mode)` beside
   `state_availability_for` — adds the interval-frontier structure iff the effective mode is not
   `stateless`; keeps the existing per-backend structures unchanged.
5. `smelt-db` `diagnostics_types.rs`: add `DiagnosticCode::DeclaredContractRequiresState`.
6. `smelt-db` `check_file_diagnostics`: after the existing `ContractDeferralInvalid` checks, run
   the validator with the effective posture (project `state.mode` narrowed by `metadata.state`)
   and each of `project_active_backends`; accumulate Error-severity diagnostics.
7. Fixture: `examples/timeseries/smelt.yml` gains `state:\n  mode: intervals` — the example
   declares a deferral window, so under the doctrine it must carry the state that measures it.
   Re-run the example gates; fix any other fixture the sweep surfaces the same way.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full).
- `cargo test -p smelt-logical --test contract_state_requirements --test contract_lattice_spec`
- `cargo test -p smelt-db --test contract_deferral_diagnostics --test integration`
- `cargo test -p smelt-cli --test example_diagnostics --test maintenance_conformance`
- `cargo test -p smelt-lsp --test example_workspaces` (example fixtures changed).
- `cargo test -p smelt-runtime --test contract_deferral_skip_e2e` plus the new e2e test.

## Commit message

`feat(state): refuse a declared contract point whose state structure is unavailable`
