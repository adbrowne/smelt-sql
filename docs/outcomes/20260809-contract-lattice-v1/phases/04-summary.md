# Phase 4 summary — the `deferral` triple

## Shipped

- `contract.deferral` (model-level) and `contract.cells[]` (`columns`/`on`/`deferral`) now parse —
  `smelt-core/src/config.rs`'s `ContractConfig`/`ContractCellConfig`. `deny_unknown_fields` no
  longer refuses them.
- Fail-loud format validation: `MetadataError::ContractDeferralInvalid`, disambiguated from
  `ContractFrozenHorizonInvalid` by walking the raw `contract:` mapping to find which field's
  value is itself unparseable (`classify_contract_data_latency_error`,
  `crates/smelt-core/src/metadata.rs`) — serde_yaml's custom-error text carries no field path at
  this struct depth.
- The single-owner triple in `crates/smelt-logical/src/contract/deferral.rs`: `validate_deferral`
  (clock-admissibility), `measure_lag`/`within_deferral` (the pure lag oracle), and
  `deferral_violations` (the probe comparison). `contract/mod.rs` now says `deferral`'s triple has
  landed.
- `smelt-db`: `DiagnosticCode::ContractDeferralInvalid`, its `map_metadata_error_to_diagnostic` arm,
  and a `check_file_diagnostics` block resolving clock-admissibility — model-level from
  `metadata.timeseries`, cell-level by resolving `cells[].on` against the model's refs and checking
  `SourceInfo.timeseries`/`mutation_profile`. `smelt-lsp`'s `backend.rs` code-string match updated.
- `smelt-runtime/src/contract_probes.rs`: `deferral_probes` (pure, opt-in builder) and
  `evaluate_deferral` (reads `IntervalStore`'s per-model latest interval end and
  `LandedDeltaStore`'s per-source latest covered end, no SQL — the probe is a pure ledger
  comparison), wired into `execute.rs` at the same pre-write site as the `frozen_horizon` probe.
- Spec: `incremental_models.md` §"Contract relaxations" states the clock-admissibility rule
  precisely (`on: backfill`/unclocked/`mutable_snapshot` all raise `ContractDeferralInvalid`);
  §"The contract lattice" names the two concrete frontiers; the Known Divergence bullet is
  narrowed to conformance parameterisation, `explain`, and the two licensed capabilities (run
  skipping, work subsumption — still phase 5). `diagnostics.md`'s divergence note updated to match.
- A `deferral`-declaring model added to the timeseries example fixtures
  (`daily_event_counts_deferred.sql`, sibling to the phase-2 `_frozen.sql` fixture, not an edit to
  it).
- New `deferral_triple_is_complete` test in `contract_lattice_spec.rs` (test 12 of the plan).

## Decisions

- Clock-admissibility for cells is resolved by walking `smelt_logical::collect_path_refs` +
  `ref_source_info` inside `check_file_diagnostics` (same pattern `maintenance_plan` already uses),
  not by a new cross-file Salsa query — kept the resolution local to the one call site that needs it.
- `deferral_probes`/`evaluate_deferral` operate at model granularity this phase, mirroring
  `frozen_horizon_probes`'s own granularity — `contract.cells[].deferral`'s per-cell probe scoping
  is folded into phase 5's scheduling work (a per-cell lag probe without per-cell scheduling has no
  consumer yet).

## For the next planner

- Phase 5 (run skipping + work subsumption) can build directly on `deferral_violations` and the two
  frontier reads already wired in `execute.rs` — the frontiers are event-time and ledger-derived as
  designed, no new state needed.
- `contract.cells[].deferral` parses and validates but its value is not yet read by any probe —
  `deferral_probes`/`evaluate_deferral` only consult the model-level `contract.deferral`. Phase 5
  should decide whether cell-level probing/scheduling reuses this model-level probe or needs its own
  per-cell frontier (a cell's own maintained frontier isn't yet tracked separately from the model's).
- Not exercised end-to-end: no `execute_project`-driven test hits the real `execute.rs` deferral
  call site (unit-level dispatch tests only, mirroring phase 3's own posture for the frozen-horizon
  probe).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`)
- `cargo test -p smelt-logical --test contract_lattice_spec` — 10 passed
- `cargo test -p smelt-db --test contract_deferral_diagnostics` — 4 passed
- `cargo test -p smelt-runtime --test contract_deferral_probe` — 4 passed
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp --test contract_late_arrival_probe` — 2 + 3 passed (unregressed)
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed
- `cargo test -p smelt-runtime --test statement_parity` — 23 passed (unregressed)
- `cargo test -p smelt-lsp --test example_workspaces` — 34 passed
