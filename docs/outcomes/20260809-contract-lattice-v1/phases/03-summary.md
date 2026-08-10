# Phase 3 summary — Late-arrival diagnostic outside the frozen horizon

**Shipped:**
- The frozen-horizon triple is complete: `smelt-logical/src/contract/frozen_horizon.rs` gains
  `frozen_band_before` (the `end - h` derivation, now shared by `clamp_write_range`),
  `PartitionCount`/`LateArrival`/`late_arrivals` (the pure baseline-comparison), and
  `emit_frozen_band_snapshot` (the per-partition row-count `SELECT` over the frozen band, both
  dialects) — 6 new unit tests, all passing.
- `crates/smelt-state/src/frozen_band_baselines.rs` — a dedicated `FrozenBandBaselineStore`
  (`.smelt/targets/<target>/frozen_band_baselines.json`), wired into `FileStore::load/save_
  frozen_band_baselines`, registered in `smelt-state/CLAUDE.md`'s inventory list.
- `crates/smelt-runtime/src/contract_probes.rs` (new) — `frozen_horizon_probes` (pure builder,
  opt-in on `contract.frozen_horizon`, resolves the model's clocked sources) and
  `dispatch_and_record_frozen_horizon_probes` (executes the snapshot, compares via
  `late_arrivals`, returns refreshed baselines + violations as data, not an early `Err` — see
  Decisions).
- Wired into `execute.rs`'s incremental-batch pre-write site, next to the source-posture probe,
  under the same `state_io_lock`/cadence policy.
- 3 new integration tests (`tests/contract_late_arrival_probe.rs`): first-run establishes
  without violating, a genuine late arrival into an already-frozen partition raises with
  partition/added-rows/`H` in the message, and the probe is opt-in only.
- Spec: `incremental_models.md`'s frozen-horizon paragraph now describes the baseline-comparative
  observable behaviour (not "counts scanned rows"); the Known Divergence bullet narrows to name
  only `deferral`, conformance parameterisation, and `explain`; `diagnostics.md`'s divergence
  bullet and both diagnostic-table rows updated to match.
- `contract_lattice_spec.rs` gained `frozen_horizon_triple_is_complete`, asserting all three legs
  exist and the module doc no longer says "lands in phase 3".

**Decisions:**
- The dispatch function never returns `Err` on a violation. It returns a
  `FrozenHorizonDispatchResult { refreshed, records, violations }` — the caller persists
  `refreshed` unconditionally, then fails the run from `violations` if non-empty. This is the
  only way to honor the plan's "refresh the baseline after a held verification and after a
  reported violation" requirement: an early `Err` (the `source_probes`/`model_probes` pattern)
  would skip the persistence step entirely, silently *keeping* the append-only-posture-style
  "never auto-heal on violation" semantics the plan explicitly rejected for this point.
- `bare_source_address`/`resolve_model_source_infos` are duplicated from `source_probes.rs`
  rather than shared — `source_probes.rs`'s own doc comment already establishes this as the
  precedent ("kept local to this module") for exactly this small pure helper pair.

**For the next planner:**
- Phase 4 (`deferral`) is next per the outcome table; nothing from this phase's own scope was
  deferred.
- Not exercised: the frozen-horizon probe is wired only at the incremental-batch site, not the
  full-refresh site — `frozen_horizon` requires `grain: partition` + `timeseries:` + batched
  incremental config (validated in phase 2), so a full-refresh run never reaches a
  `frozen_horizon`-declared model in practice; this mirrors how the write-clamp in
  `build_model_plans` is also batch-only. Worth a one-line confirmation in the spec if a future
  phase finds a full-refresh path that *can* reach a `frozen_horizon` model.
- The runtime integration tests call `contract_probes` functions directly (mirroring
  `source_probes.rs`'s test harness) rather than driving a full `execute_project` run — faster
  and sufficient for this phase's oracle (pure builder + dispatch), but the `execute.rs` wiring
  itself has no end-to-end test exercising the real pre-write call site. If a future phase needs
  that confidence, `contract_frozen_horizon_clamp.rs`'s `execute_project`-driven harness is the
  template, but it would need a real clocked source (that harness currently uses a
  source-free `VALUES` model).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo
  test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_lattice_spec` — 9 passed.
- `cargo test -p smelt-runtime --test contract_late_arrival_probe` — 3 passed.
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp` — 2 passed (unregressed).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — passed.
- `cargo test -p smelt-runtime --test statement_parity` — 23 passed (new emitter authored only
  in `smelt-logical`).
