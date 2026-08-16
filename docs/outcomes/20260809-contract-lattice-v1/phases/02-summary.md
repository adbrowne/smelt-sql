# Phase 2 summary — `frozen_horizon:` declaration, validation, write-eligibility clamp

**Shipped:**
- `ContractConfig { frozen_horizon: Option<DataLatency> }` in `smelt-core::config`
  (`deny_unknown_fields`); `ModelMetadata.contract: Option<ContractConfig>`; `"contract"`
  registered in the frontmatter catalogue.
- `MetadataError::ContractFrozenHorizonInvalid { why }`, raised by `extract_single_model`'s
  strict `contract:` pre-validation (mirrors the `reuse`/`state` pattern) — an unparseable
  `frozen_horizon` is a dedicated error, never a generic `YamlParseError`; a `deferral:` or
  other unknown key under `contract:` is a loud unknown-field parse error.
- `crates/smelt-logical/src/contract/{mod.rs,frozen_horizon.rs}`: `validate_frozen_horizon(grain)`
  (grain-admissibility) and `clamp_write_range(start, end, h)` (pure, unit-agnostic narrowing
  transform, `start' = max(start, end - h)`), both unit-tested. Module doc records which legs
  land when (probe emitter is phase 3; `deferral` is phase 4).
- `DiagnosticCode::ContractFrozenHorizonInvalid`, wired in `smelt-db::check_file_diagnostics`
  from two call sites sharing the one code: the frontmatter-parse-time format error, and a new
  grain-admissibility check block calling `smelt_logical::validate_frozen_horizon`.
- Write-clamp wired into `smelt-runtime::execute::build_model_plans`: `full_range.start` is
  narrowed via `clamp_frozen_horizon_write_range` before `compute_incremental_windows_ordered`,
  logged at `info` when it actually narrows.
- Spec deltas: `incremental_models.md` Known Divergences narrowed to name what's built vs.
  remaining; `diagnostics.md` Known Divergences updated (`ContractFrozenHorizonInvalid` now has
  a live derivation site).
- New example fixture `examples/timeseries/models/daily_event_counts_frozen.sql` (dedicated
  model, not an edit to a shared golden-fixture model) demonstrating the clean declaration path.

**Decisions:**
- Format validation happens at frontmatter-parse time in `smelt-core` (no grain needed);
  grain-admissibility happens later in `smelt-db`/`smelt-logical` (needs `ModelMetadata.grain`).
  Both diagnostics share `DiagnosticCode::ContractFrozenHorizonInvalid` per the spec's "unparseable
  or negative, or declared on a non-partition-grain model" single-code framing.
- `ContractConfig`'s pre-validation is done by attempting `serde_yaml::from_value::<ContractConfig>`
  and classifying the error by message content (`"invalid data_latency"` → dedicated variant,
  else → generic `YamlParseError`) rather than a duplicate raw-string struct — avoids a second
  shape to keep in sync.
- Did **not** reuse `daily_events.sql` (an existing partition-grain example) for the
  `frozen_horizon:` fixture — it's a golden-fixture input for `crates/smelt-cli/tests/explain.rs`
  and adding `contract:` changed its `--show-sql` scan-clamp rendering, breaking an unrelated
  pinned snapshot. Added a new dedicated model instead.

**For the next planner:**
- Phase 3 (late-arrival diagnostic) can reuse `validate_frozen_horizon`'s grain check and the
  same `ContractFrozenHorizonInvalid`-adjacent code path; the probe emitter is unbuilt as noted
  in `contract/mod.rs`'s module doc.
- `smelt explain` does not yet render the effective contract (phase 6) — `daily_event_counts_frozen`
  is available as a fixture once that lands.
- Not addressed (deliberately out of this phase's scope per the plan): `deferral:`, per-cell
  `cells:` refinement, `maintenance_conformance` parameterisation.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-core --test contract_frozen_horizon` — 3 passed.
- `cargo test -p smelt-logical --test contract_lattice_spec` — 8 passed.
- `cargo test -p smelt-db --test contract_frozen_horizon_diagnostics` — 2 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed.
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp` — 2 passed.
