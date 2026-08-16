# Phase 5 plan — deferral-licensed run skipping and ledger-proven work subsumption

## Objective

Turn the landed `deferral` triple into the two capabilities it licenses: a run whose pending
input set is inside the declared window is **skipped** (recorded, not silently dropped), and a
previously-deferred window folded into a later, wider run is recorded as **subsumed**, proven
from the ledger. Advances success criterion 2, and closes the phase-3/4 carry-forward that no
`execute_project`-driven test hits the live deferral path.

## Design decisions this phase makes

- **Skip rule.** With `contract.deferral: D`, both frontiers known, and measured lag in
  `0 < lag ≤ D`, the model's run is skipped. `lag ≤ 0` (nothing pending) and a missing frontier
  fall through to the normal path — skipping is a licensed relaxation, never the fallback.
- **A deferral skip propagates to dependents.** A dependent that ran while its upstream was
  deferred would record interval coverage for a window its upstream never folded, and would never
  revisit it — a silent hole the default point forbids. Dependents of a deferral-skipped model are
  therefore skipped too (`skipped_deferral_upstream`), computed as a fixpoint over `upstream_map`
  exactly as `resume_skip_set` is.
- **Subsumption proof.** The pending window is `(maintained_frontier, input_frontier]`, read from
  the same two ledger frontiers the probe already uses. It is *subsumed* when a prior run manifest
  for this model recorded `skipped_deferral` **and** this run's write range covers the pending
  window. Both legs are ledger facts; neither is inferred.
- **Model granularity.** Scheduling reads the model-level `contract.deferral`; per-cell refinement
  keeps validating fail-loud but is not scheduled (see the outcome's Out of scope).

## Spec delta (made first, by the implement step)

`docs/specs/incremental_models.md`:

- §"The contract lattice", the Deferral (`D`) paragraph: state the two capabilities operationally
  — the skip rule above, that a skipped run is recorded in the run manifest (`skipped_deferral`)
  rather than silently omitted, that dependents of a skipped cell are skipped with it, and the
  two-legged subsumption proof (prior recorded deferral skip + current write range covering the
  pending window), recorded on the covering run's manifest record.
- §Known Divergences, the contract-lattice bullet: delete "run skipping, work subsumption" from
  the still-missing list; the bullet now names only conformance parameterisation and `explain`.

`docs/specs/run_state.md` §Surface (the manifest record): add the two new `strategy` values and the
optional `subsumed` field to the recorded shape.

## Tests (red → green)

`crates/smelt-logical/src/contract/deferral.rs` (unit):
1. `run_license_skips_when_lag_is_within_d` — `0 < lag ≤ D` yields `Skip { lag, d }`.
2. `run_license_runs_when_lag_exceeds_d` — `lag > D` yields `Run`.
3. `run_license_runs_when_nothing_is_pending` — `lag ≤ 0`, and either frontier `None`, yield `Run`.
4. `pending_window_is_maintained_exclusive_to_input_inclusive` — window shape, `None` when `lag ≤ 0`.
5. `subsumption_requires_a_covering_scheduled_range` — pending window not covered → `None`.
6. `subsumption_requires_a_prior_deferred_run` — no recorded prior skip → `None`, even when covered.

`crates/smelt-logical/tests/contract_lattice_spec.rs`:
7. `deferral_capabilities_are_single_owned` — the licensing decision (lag-vs-`D`, window coverage)
   resolves in `smelt_logical::contract::deferral`; `smelt-runtime` carries no independent
   comparison of the two frontiers or of the pending/scheduled ranges.

`crates/smelt-runtime/tests/contract_deferral_schedule.rs` (new, unit over the pure builder):
8. `undeclared_model_is_never_deferral_skipped` — no `contract.deferral` → always `Run`.
9. `declaring_model_within_window_is_skipped`.
10. `deferral_skip_propagates_to_dependents` — closure over the upstream map.
11. `covering_run_after_a_recorded_skip_reports_the_subsumed_window`.

`crates/smelt-runtime/tests/contract_deferral_skip_e2e.rs` (new, real DuckDB through
`execute_project`; move to `smelt-cli` if a project fixture is needed):
12. `deferred_run_is_recorded_skipped_and_writes_nothing` — manifest record `skipped_deferral`,
    `outcome: Skipped`, target table row count unchanged, no new interval recorded.
13. `catch_up_run_records_the_subsumed_window` — a later run covering the pending window records
    `subsumed` on its manifest record.

## Tasks

1. Spec edits above (spec-first).
2. `smelt-logical::contract::deferral`: add `RunLicense`, `run_license`, `pending_window`,
   `SubsumedWork`, `subsumption` with doc comments naming them as the deferral point's licensing
   half; extend the module doc.
3. `smelt-state`: add `#[serde(default, skip_serializing_if = "Option::is_none")] pub subsumed:
   Option<SubsumedWindow>` to `ModelRunRecord` (dated pending/covering bounds), fix construction
   sites.
4. `smelt-runtime`: a pure per-model decision builder (in `contract_probes.rs` or a sibling
   `contract_schedule.rs`) mapping model metadata + `IntervalStore`/`LandedDeltaStore` frontiers +
   `prior_runs` + the run's write range to `RunLicense`/`SubsumedWork` — thin over the
   `smelt-logical` functions, no comparison of its own.
5. `execute.rs`: widen the `upstream_map` build condition to include "some selected model declares
   `contract.deferral`"; build the deferral decision map beside `resume_skip_set` (one state load,
   dependents closed over `upstream_map`); add the skip block after the resume-skip block
   (`skipped_deferral` / `skipped_deferral_upstream`, `RunOutcomeKind::Skipped`, reporter
   `model_completed`, no compilation or backend call); attach `subsumed` to the manifest record on
   the covering run.
6. Reporter/log line naming the model, the measured lag, and `D` so a skip is visible in a run.
7. Add a catch-up fixture only if the existing `daily_event_counts_deferred.sql` cannot exercise
   both e2e cases; do not edit golden-fixture models.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_skip_e2e`
- `cargo test -p smelt-runtime --test contract_deferral_probe --test contract_frozen_horizon_clamp --test contract_late_arrival_probe` (unregressed)
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test probe_manifest`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(contract-lattice): deferral-licensed run skipping and ledger-proven subsumption`
