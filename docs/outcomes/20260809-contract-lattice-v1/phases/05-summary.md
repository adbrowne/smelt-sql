# Phase 5 summary — deferral-licensed run skipping and ledger-proven work subsumption

**Shipped:**
- `smelt_logical::contract::deferral` gained the licensing half of the triple: `RunLicense`,
  `run_license`, `PendingWindow`, `pending_window`, `SubsumedWork`, `subsumption` — pure functions,
  single-owning the skip-vs-run decision and the subsumption proof
  (`crates/smelt-logical/src/contract/deferral.rs`).
- `smelt-runtime::contract_probes` gained the scheduling builders thin over those functions:
  `deferral_decision` (per-model skip/pending decision from the two ledger frontiers),
  `propagate_deferral_skip` (fixpoint closure over `upstream_map`), and `subsumed_window` (date-
  formatting wrapper over `subsumption`).
- `execute.rs` wires it in: a `deferral_own_skip`/`deferral_pending` snapshot computed once before
  the wavefront scheduler; `deferral_skip_set` closes it over dependents; a new skip block
  (mirroring `--resume`'s) records `skipped_deferral`/`skipped_deferral_upstream` with no
  compilation, no backend call, no ledger write; the incremental-batch manifest site attaches
  `subsumed` when a prior manifest recorded `skipped_deferral` for the model and this run's write
  range covers the pending window.
- `smelt_state::ModelRunRecord.subsumed: Option<SubsumedWindow>` (new field, `#[serde(default)]`).
- Spec: `docs/specs/incremental_models.md` §"The contract lattice" states the skip rule, manifest
  recording, dependent propagation, and two-legged subsumption proof operationally; the Known
  Divergence bullet no longer lists the two capabilities as missing. `docs/specs/run_state.md`
  §"Run manifest" documents the two new `strategy` values and the `subsumed` field.
- Tests: 12 unit tests in `deferral.rs` (6 new), `contract_lattice_spec.rs`'s
  `deferral_capabilities_are_single_owned` (structural: `smelt-runtime` calls the shared functions,
  never reimplements the lag-vs-window comparison), 5 pure-builder tests in the new
  `contract_deferral_schedule.rs`, and 2 real-DuckDB `execute_project`-driven tests in the new
  `contract_deferral_skip_e2e.rs`.

**Decisions:**
- All three design decisions from the plan (skip propagates to dependents; subsumption needs both
  ledger legs; model granularity only) are implemented exactly as decided — see outcome.md's
  existing 2026-08-10 phase-5-plan entries.
- **Discovery not anticipated by the plan**: under the default `PerRun` probe cadence, a run whose
  lag has genuinely exceeded `D` will *always* also trip the phase-4 `ContractDeferralExceeded`
  probe, since both read the identical stale pre-write ledger frontier. This is correct — a
  `lag > D` state that persisted long enough for a run to observe it really is a contract
  violation — but it means the e2e "catch-up run records `subsumed`" scenario can only be
  demonstrated with the probe's cadence set to `off` for that fixture (`contract_deferral_skip_
  e2e.rs`'s `stage_project` doc comment explains this). A production deployment licensing deferral
  needs to tune `D` and the probe cadence together; this is not a code gap, just a fixture
  necessity, documented inline rather than filed as a new divergence.

**For the next planner:**
- Phase 6 (conformance oracle parameterised per lattice point) and phase 7 (`explain` rendering,
  docs-site) are unaffected by this phase's scope and remain next.
- Nothing was deferred out of this phase's own scope.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_lattice_spec` — 11 passed.
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_skip_e2e`
  — 5 + 2 passed.
- `cargo test -p smelt-runtime --test contract_deferral_probe --test contract_frozen_horizon_clamp --test contract_late_arrival_probe`
  — unregressed (4 + 2 + 3 passed).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test probe_manifest`
  — 4 + 3 + 23 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 67 passed.
