# Outcome: Contract lattice v1 — frozen horizons and deferral

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §4.2, §6 step 5; `docs/research/20260726-beyond-ivm-differentiation.md` §5.2/§5.3
**Spec anchors:** `docs/specs/incremental_models.md` (the equivalence invariant)

## The outcome

The equivalence invariant becomes the *default* point in a small declared
contract lattice. v1 ships the two relaxations with the clearest oracles:
**frozen horizon** (partitions older than H are never revisited; late data
outside H is diagnosed, not silently excluded — closing the one accepted
silent-data behaviour in the family) and **deferral** (a cell may lag its
inputs by up to D, licensing run skipping and work subsumption). Each
relaxation is declared, validated, probe-checked, and printed by
`smelt explain`; the conformance oracle is parameterised per lattice point.

## Success criteria (checkable)

1. `frozen_horizon:` declared on a partition-grain model: writes outside H
   are clamped by contract (not merely by derived reach), and a genuinely
   late arrival outside H raises a named diagnostic instead of being silently
   excluded (deleting that Known Divergence).
2. `deferral:` declared on a cell/model: a run whose pending input set is
   within the deferral window may be skipped, and a pending small run implied
   by a scheduled larger one is subsumed (the ledger proves the subsumption).
3. The spec defines the lattice: the default contract, the two v1 points,
   each with its restated oracle and its probe; declarations compose with the
   existing shape facts without new modes.
4. `smelt explain` prints the effective contract per cell — default or
   relaxed, with the relaxation's parameters.
5. `maintenance_conformance` is parameterised by lattice point: relaxed cells
   are asserted against their *relaxed* oracle, default cells against strict
   equivalence; a relaxation is never silently tested as the default.
6. All standing gates green.

## Out of scope

- Other lattice points (reconciliation points, declared indifference,
  per-column-group freshness, retention) — v2+ once these two prove the shape.
- Restating the invariant per-cell in the spec headline (that is the spec
  redraft outcome's job; v1 adds the lattice without rewriting the whole spec).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: the lattice framing — default point, frozen horizon, deferral; oracles and probes per point | done |
| 2 | `frozen_horizon:` declaration, validation, write-eligibility clamp wiring | done |
| 3 | Late-arrival diagnostic outside the frozen horizon (delete the silent exclusion) | done |
| 4 | `deferral:` declaration + validation + the ledger-derived lag oracle and `ContractDeferralExceeded` probe (the deferral triple) | done |
| 5 | Deferral-licensed run skipping and ledger-proven work subsumption | pending |
| 6 | Conformance oracle parameterised per lattice point + recipes for both relaxations | pending |
| 7 | Surface: explain contract rendering, docs-site update | pending |

## Decision log

- 2026-08-09 — **Oracle home settled** (rethink §6 open question 2, settled with Andrew): a lattice point is admissible only as a complete single-owner definition in `smelt-logical` — (declaration schema, pure oracle transform, probe emitter). The conformance gate consumes the oracle transform rather than encoding its own comparator; runtime probes emit from the same definition. This makes the admission rule ("what does the oracle become, and what probe checks it") structural, mirroring the statement-emission single-owner rule. Harness-local comparators (drift risk) and ad-hoc probes (reopens declared-but-unchecked) were rejected.

- 2026-08-10 — **Relaxation surface is a top-level `contract:` block**, not an extension of `maintenance:` (phase 1 plan): `maintenance:` is specified as never widening what admission allows, and a lattice point does exactly that; `contract:` carries model-level `frozen_horizon:`/`deferral:` plus optional per-cell refinement addressed like `maintenance.cells`. `horizon_ceiling:` is untouched and stays a warning threshold on the derived horizon.
- 2026-08-10 — Phase table unchanged (no prior phase summary to reshape against; phase 1 is the outcome's first phase).
- 2026-08-10 — Phase 1 done: spec lands §"The contract lattice" (Semantics) + §"Contract relaxations (`contract:`)" (Surface) in `docs/specs/incremental_models.md`, the four diagnostic codes cross-catalogued in `diagnostics.md`, the single-owner constraint + `CLAUDE.md` bullet, the Known Divergence, and the standing gate `crates/smelt-logical/tests/contract_lattice_spec.rs`.
- 2026-08-10 — Phase table unchanged again (phase 1's summary surfaced no work needing a new row; its only carry-forward was a local DuckDB env-path fact, environment not outcome scope).
- 2026-08-10 — **Layering split for the single-owner triple** (phase 2 plan): the `contract:` *serde shape* (`ContractConfig`) lives in `smelt-core::config` beside `MaintenanceConfig` because `ModelMetadata` must deserialize it and `smelt-core` sits below `smelt-logical`; validation, the oracle transform, and the probe emitter — every semantic leg — are single-owned in a new `smelt-logical/src/contract/`. The single-owner rule binds the semantics, not the struct's crate.
- 2026-08-10 — **`deferral:` and `contract.cells[]` are refused with a loud parse error until phase 4** wires their validation, rather than parsed-and-ignored: an accepted-but-unenforced relaxation key is exactly the silent weakening the lattice exists to prevent.
- 2026-08-10 — **Frozen-horizon clamp anchors on the run's end date**, floor `end − H`, narrowing only (`start' = max(start, end − H)`) — deterministic, and never widens the derived reach clamp.

- 2026-08-10 — Phase 2 done: `contract.frozen_horizon` declaration (`ContractConfig` in
  `smelt-core`), fail-loud format validation (`MetadataError::ContractFrozenHorizonInvalid`),
  grain-admissibility validation and the pure write-range clamp (`smelt-logical/src/contract/`),
  the `DiagnosticCode::ContractFrozenHorizonInvalid` wiring in `smelt-db`, and the clamp wired
  into `smelt-runtime::execute::build_model_plans`. `deferral:`/`cells:` remain refused
  fail-loud. Added a dedicated new example fixture rather than editing an existing
  golden-fixture model (avoided an unrelated `explain.rs` snapshot break).

- 2026-08-10 — **Lateness is observed by a frozen-band per-partition baseline, not by
  scanning** (phase 3 plan): a partition-filtered scan never reads an already-frozen row, so the
  probe follows `source_probes.rs`'s Establish/Verify shape — each run snapshots per-partition
  row counts of the model's clocked sources over the band before `end − H`, and a later
  count increase (or a wholly new frozen partition) is the genuine late arrival. The
  landed-delta ledger was rejected as the signal: its v1 entries are the run's own already-clamped
  window, not an arrival record. The spec's "counts scanned rows" wording is corrected accordingly
  (spec-first edit in the phase). Baselines live in a dedicated `frozen_band_baselines.json`, not
  the append-only posture store, whose refresh rule differs and would cross-talk.
- 2026-08-10 — Phase table unchanged (phase 2's summary surfaced no success-criteria work outside
  the existing rows; its carry-forwards — the unbuilt probe emitter and the missing `explain`
  rendering — are already rows 3 and 6).

- 2026-08-10 — Phase 3 done: the frozen-horizon triple is complete. `smelt-logical::contract::
  frozen_horizon` gained the pure baseline comparison (`late_arrivals`) and probe emitter
  (`emit_frozen_band_snapshot`); a dedicated `frozen_band_baselines.json` state store
  (`smelt-state`); `smelt-runtime::contract_probes` (pure builder + dispatch), wired into
  `execute.rs`'s incremental-batch pre-write site. **Design decision**: the dispatch never
  returns `Err` on a violation — it returns refreshed baselines + a `violations` list as data, so
  the caller persists the baseline unconditionally before failing the run, satisfying "report
  once, not every subsequent run" (a plain early-`Err`, the `source_probes`/`model_probes`
  pattern, would have skipped persistence on violation). Not yet exercised: no end-to-end
  `execute_project`-driven test hits the real `execute.rs` call site (unit-level dispatch tests
  only, mirroring `source_probes.rs`'s own test posture).

- 2026-08-10 — **Phase 4 split into two rows** (phase 4 plan): the original row bundled the
  deferral *triple* (declaration schema, oracle transform, probe emitter — the admission rule the
  outcome's own decision log makes structural) with the two *capabilities* the point licenses (run
  skipping, work subsumption). They are separable and each is a phase's worth of work: the triple
  is a compile-time + ledger-comparison layer in `smelt-logical`/`smelt-db`, the capabilities are a
  scheduling change in `smelt-runtime`'s execute loop, and the capabilities are only safe to build
  once the oracle they must not violate exists. Nothing left the outcome; success criterion 2 is
  now served by rows 4 and 5 together. Old rows 5/6 shift to 6/7.
- 2026-08-10 — **Deferral lag is event-time, read from the existing ledger stores** (phase 4 plan):
  the maintained frontier is `IntervalStore`'s per-model `latest_date()` and the input frontier is
  `LandedDeltaStore`'s per-source max covered end — both already written by `execute.rs`. No new
  state file and no wall-clock arrival record is introduced; the spec's "ledger-derived" probe is
  literally a comparison of two frontiers the ledger already holds.

- 2026-08-10 — Phase 4 done: the `deferral` triple is complete —
  `contract.deferral`/`contract.cells[]` parse and fail-loud validate
  (`ContractDeferralInvalid`, disambiguated from `ContractFrozenHorizonInvalid` by walking the raw
  YAML mapping rather than the error text), the pure lag oracle and probe comparison land in
  `smelt-logical::contract::deferral`, and `smelt-runtime::contract_probes::evaluate_deferral`
  dispatches at the same pre-write site as `frozen_horizon`'s probe, reading two already-recorded
  ledger frontiers (`IntervalStore`, `LandedDeltaStore`) rather than executing SQL. Model-level
  granularity only this phase — cell-level `deferral` validates but is not yet probed;
  `contract.cells[].deferral`'s own probe/scheduling is phase 5 (`docs/outcomes/
  20260809-contract-lattice-v1/phases/04-summary.md`).

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
