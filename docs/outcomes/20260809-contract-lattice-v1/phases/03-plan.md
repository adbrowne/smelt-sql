# Phase 3 plan — Late-arrival diagnostic outside the frozen horizon

## Objective

Land the third leg of the `frozen_horizon` triple: the **probe emitter** and its
live dispatch, so a genuinely late arrival landing outside `H` raises
`ContractLateArrivalOutsideHorizon` instead of being silently excluded. This
completes success criterion 1 (the half phase 2 left open) and deletes the
default point's one accepted silent-data behaviour *for opted-in models*.

## Design constraint: how lateness is observed

A partition-filtered scan never reads a row whose partition is already frozen —
"smelt cannot fail loud on a row it never scans". So the probe is **baseline-
comparative**, mirroring `source_probes.rs`'s Establish/Verify shape: each run of
a `frozen_horizon` model snapshots per-partition row counts of its clocked
sources **over the frozen band** (partitions strictly before `end − H`) into a
dedicated store; the next run compares. A frozen-band partition whose count
**increased**, or that is wholly new versus the baseline, is a genuine late
arrival. First observation establishes, never verifies. The landed-delta ledger
is *not* usable here — its v1 entries are the run's own (already clamped) window,
not an arrival signal.

Store isolation: a dedicated `frozen_band_baselines.json` reusing
`smelt_state::source_postures::SourcePostureStore`'s shape, **not** the
append-only posture store — the two have different refresh rules (posture
refreshes only when held) and must not cross-talk.

## Spec delta

`docs/specs/incremental_models.md` §"The contract lattice", frozen-horizon
paragraph: replace "the probe counts scanned rows whose natural partition falls
outside `H`" with the observable behaviour above — the probe compares a recorded
per-partition frozen-band baseline of the model's clocked sources against their
current state, and raises `ContractLateArrivalOutsideHorizon` naming the
partition, the added row count, and the declared `H`; the first run of a
`frozen_horizon` model establishes the baseline and verifies nothing. Narrow the
Known Divergence bullet under §"The contract, plan, and graph layer" to name only
what remains (`deferral`, conformance parameterisation, `explain`).
`docs/specs/diagnostics.md` §Known Divergences: `ContractLateArrivalOutsideHorizon`
now has a live emitter/derivation site; leave the other two contract codes listed.

## Tests (red-green)

- `smelt-logical` unit (`contract/frozen_horizon.rs`):
  - `frozen_band_before_is_end_minus_h` — the band boundary shares one derivation
    with `clamp_write_range` (no second copy of `end − H`).
  - `late_arrivals_flags_count_increase_in_frozen_band` — baseline 100 → current
    120 on a frozen partition yields one arrival with `added_rows: 20`.
  - `late_arrivals_flags_partition_absent_from_baseline` — a frozen partition
    present now, absent from the baseline, is an arrival.
  - `late_arrivals_ignores_partitions_inside_horizon` — changes at/after
    `end − H` are not arrivals (that is ordinary maintenance).
  - `late_arrivals_ignores_count_decrease` — a shrink is not a late *arrival*
    (out of this point's scope; not silently upgraded to a violation).
  - `emit_frozen_band_snapshot_counts_per_partition` — emitted SQL groups by the
    partition column, filters `< frozen_before`, for both dialects.
- `smelt-runtime` integration (`tests/contract_late_arrival_probe.rs`):
  - `first_run_establishes_frozen_band_baseline_without_violating` — no
    diagnostic, store written.
  - `late_row_in_frozen_band_raises_contract_late_arrival_outside_horizon` — stage
    a source, run, insert a row into a frozen partition, re-run; the run fails
    with the probe code naming the partition and `H`.
  - `no_frozen_horizon_declaration_dispatches_no_contract_probe` — the probe is
    opt-in only.
- `smelt-db` integration: the existing diagnostics-catalogue leg still passes with
  the code now live (mirror however `DeclaredMonotonicityViolated` is catalogued).
- `smelt-logical --test contract_lattice_spec` — extend the standing gate to
  assert the frozen-horizon triple now has all three legs present.

## Tasks

1. Spec edits above (spec-first, before code).
2. `smelt-logical/src/contract/frozen_horizon.rs`: `frozen_band_before`,
   `LateArrival { partition, added_rows }`, `late_arrivals(baseline, current,
   frozen_before)` — pure, string-ISO partition compare, unit-tested.
3. Same module: `emit_frozen_band_snapshot(source_table, partition_column,
   frozen_before, dialect) -> MaintenanceStatement`, reusing the shared probe
   result-shape helpers in `maintenance/emit.rs` (widen them to `pub(crate)` if
   private rather than duplicating the shape).
4. `smelt-state`: `frozen_band_baselines.json` load/save on `FileStore` reusing
   `SourcePostureStore`; register the filename in the state-file inventory list
   alongside the other stores.
5. `smelt-runtime/src/contract_probes.rs` (new): pure build of the per-source
   Establish/Verify action for a model declaring `contract.frozen_horizon`, then
   dispatch through `crate::probes::dispatch_probe` with `ProbeContext {
   probe_code: "ContractLateArrivalOutsideHorizon", fact: "contract.frozen_horizon",
   .. }` and the same violation-fails-the-run behaviour as `source_probes`.
6. Wire it at the pre-write probe site in `execute.rs` next to the source-posture
   dispatch, under the same `state_io_lock` and cadence policy; refresh the
   baseline after a held verification **and** after a reported violation (the
   violation is reported once, not re-reported every subsequent run).
7. Update `crates/smelt-logical/src/contract/mod.rs`'s module doc: the probe
   emitter has landed; only `deferral` (phase 4) remains unbuilt.
8. Extend `contract_lattice_spec.rs` to assert the completed triple.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-runtime --test contract_late_arrival_probe`
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp` (unregressed)
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-runtime --test statement_parity` (new emitter is in
  `smelt-logical`, not authored by a backend)

## Commit message

`feat(contract-lattice): late-arrival probe outside the frozen horizon`
