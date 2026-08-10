# Phase 4 plan — the `deferral` triple: declaration, validation, lag oracle + probe

## Objective

Land the second lattice point's complete single-owner triple: the `contract.deferral` declaration
schema (model-level plus `contract.cells[]` refinement), its fail-loud validation
(`ContractDeferralInvalid`), and the pure ledger-derived lag oracle plus its
`ContractDeferralExceeded` probe, single-owned in `smelt-logical/src/contract/deferral.rs`. This
advances success criterion 2 (the declaration half — run skipping and subsumption are phase 5) and
criterion 3 (both v1 points declared, validated and probe-checked). `deferral:`/`cells:` stop being
refused fail-loud and start being enforced.

## Design constraints (settled; do not re-litigate)

- **Lag is event-time, not wall-clock.** Maintained frontier = `IntervalStore::get(model)
  .latest_date()`; input frontier = max covered-interval end across the model's clocked sources in
  `LandedDeltaStore`. Both stores are already written by `execute.rs`; no new state file.
- **A source with no interval representation has no lag.** `WholeTable`-posture sources
  (`mutable_snapshot`/unclocked) contribute no frontier — a cell whose only trigger is such a
  source has no clock to measure lag against and is `ContractDeferralInvalid` at compile time,
  never silently unprobed.
- **The probe reports, it does not clamp.** Exceeding `D` fails the run with a named diagnostic;
  it never narrows a window (unlike `frozen_horizon`'s clamp).
- The probe compares ledger data, so unlike `frozen_horizon` it emits no SQL — the "probe emitter"
  leg is the pure comparison function in `smelt-logical` plus its `contract_probes.rs` dispatch.

## Spec delta

`docs/specs/incremental_models.md`:
- §"Contract relaxations (`contract:`)" — replace the Known-Divergence-implied "refused until
  wired" status for `deferral`/`cells:` and state the admissibility rule precisely: model-level
  `deferral` requires the model to carry a `timeseries:` clock; a `cells[]` entry's `deferral`
  requires its `on:` trigger to be a clocked, interval-representable source (`on: backfill`, an
  unclocked source, and a `mutable_snapshot` source each raise `ContractDeferralInvalid`).
- §"The contract lattice" → **Deferral (`D`)** — one sentence making the probe's two frontiers
  concrete (maintained frontier vs input frontier, both event-time, both read from the ledger).
- §Known Divergences — narrow the remaining bullet to conformance parameterisation and `explain`
  only (drop `deferral`).

## Tests (red-green)

`crates/smelt-core/tests/` (metadata parse):
1. `contract_deferral_parses` — `contract: { deferral: '6 hours' }` deserializes into
   `ContractConfig.deferral`, no longer an unknown-field error.
2. `contract_cells_parse` — a `contract.cells[]` entry with `columns`/`on`/`deferral` deserializes.
3. `contract_deferral_unparseable_is_named_error` — `deferral: 'soonish'` raises
   `MetadataError::ContractDeferralInvalid`, **not** `ContractFrozenHorizonInvalid` and not a
   generic YAML error (guards the shared `"invalid data_latency"` string heuristic).

`crates/smelt-logical/src/contract/deferral.rs` (unit):
4. `deferral_requires_a_clock` — validation rejects a model-level `deferral` with no clock and a
   cell whose trigger has no interval representation, naming the offender; accepts the clocked case.
5. `lag_is_input_frontier_minus_maintained_frontier` — the pure oracle computes zero lag when the
   frontiers match, positive lag when the input frontier is ahead.
6. `within_deferral_admits_lag_up_to_d_inclusive` — lag ≤ `D` holds, lag > `D` violates.
7. `missing_maintained_frontier_is_not_a_violation` — a model with no recorded intervals (first
   run) establishes rather than violates.

`crates/smelt-db/tests/contract_deferral_diagnostics.rs`:
8. `deferral_without_a_clock_raises_diagnostic` — `DiagnosticCode::ContractDeferralInvalid` at the
   model, with the offending reason in the message.

`crates/smelt-runtime/tests/contract_deferral_probe.rs`:
9. `probe_holds_within_the_window` — ledger frontiers within `D` produce no violation.
10. `probe_raises_when_lag_exceeds_d` — the violation names the cell and the measured lag.
11. `probe_is_opt_in` — no `contract.deferral` ⇒ empty probe set.

`crates/smelt-logical/tests/contract_lattice_spec.rs`:
12. `deferral_triple_is_complete` — mirrors `frozen_horizon_triple_is_complete`: all three legs
    exist in `contract::deferral` and `contract/mod.rs`'s landing-status doc no longer says
    `deferral` is unbuilt.

## Tasks

1. Spec edits above, first.
2. `smelt-core/src/config.rs`: add `deferral: Option<DataLatency>` and
   `cells: Vec<ContractCellConfig>` to `ContractConfig`; new `ContractCellConfig { columns, on,
   deferral }` mirroring `MaintenanceCellConfig`'s addressing fields.
3. `smelt-core/src/metadata.rs`: add `MetadataError::ContractDeferralInvalid { why }`; in the
   `contract:` pre-validation, disambiguate the `"invalid data_latency"` failure by which key the
   value mapping carries (`frozen_horizon` vs `deferral`/`cells[].deferral`) instead of the current
   unconditional attribution to `frozen_horizon`.
4. New `smelt-logical/src/contract/deferral.rs`, re-exported from `contract/mod.rs` and
   `smelt-logical/src/lib.rs`: `validate_deferral` (clock admissibility, pure — takes whether the
   model/cell has an interval-representable clock), `DeferralLag`/`measure_lag` (the oracle
   transform over two frontier values), `within_deferral`, and `deferral_violations` (the probe
   comparison producing the cell/lag/`D` payload). Update `mod.rs`'s landing-status doc.
5. `smelt-db`: `DiagnosticCode::ContractDeferralInvalid`, the exhaustive `MetadataError` arm in
   `map_metadata_error_to_diagnostic`, and the `check_file_diagnostics` call into
   `validate_deferral` beside the existing `validate_frozen_horizon` call; catalogue the code.
6. `smelt-runtime/src/contract_probes.rs`: `deferral_probes` (pure builder, opt-in) and
   `evaluate_deferral` (reads `IntervalStore` + `LandedDeltaStore`, returns violations as data —
   same report-as-data shape phase 3 settled, not an early `Err`); wire at the same pre-write site
   in `execute.rs` under the existing `state_io_lock`/cadence policy.
7. Add a `deferral`-declaring model to the phase-2 example fixture directory (a new model file, not
   an edit to a golden-snapshot model — phase 2's lesson).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-db --test contract_deferral_diagnostics`
- `cargo test -p smelt-runtime --test contract_deferral_probe`
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp` and `--test
  contract_late_arrival_probe` (unregressed)
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-runtime --test statement_parity`

## Commit message

`feat(contract-lattice): deferral declaration, validation, and the ledger-derived lag probe`
