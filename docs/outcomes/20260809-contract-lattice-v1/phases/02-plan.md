# Phase 2 plan — `frozen_horizon:` declaration, validation, write-eligibility clamp

## Objective

Land the first half of the frozen-horizon lattice point as a real declaration: the `contract:`
frontmatter block carrying `frozen_horizon:`, its validation (`ContractFrozenHorizonInvalid`), and
the by-contract narrowing of the partition-grain write range. Advances success criterion 1 (the
clamp half; the late-arrival diagnostic is phase 3) and starts criterion 3's "declarations compose
without new modes".

## Design decisions this phase pins

- **Schema type in `smelt-core`, all semantics in `smelt-logical`.** `ContractConfig` is a pure
  serde shape next to `MaintenanceConfig` in `crates/smelt-core/src/config.rs` (layering:
  `smelt-core` is below `smelt-logical`, and `ModelMetadata` must deserialize it). Validation, the
  oracle transform, and — from phase 3 — the probe emitter are single-owned in a new
  `crates/smelt-logical/src/contract/` module, per the lattice-point single-owner rule.
- **`deferral:`/`cells:` are NOT accepted yet.** `ContractConfig` is `deny_unknown_fields` with
  `frozen_horizon` only this phase; declaring `deferral:` is a loud parse error until phase 4 wires
  its validation. Accepting an unvalidated, unenforced relaxation key would be exactly the silent
  weakening the lattice exists to prevent.
- **Clamp anchor is the run's end date** (the run's as-of), floor = `end − H`, and the clamp only
  ever narrows: `start' = max(start, end − H)`. Deterministic, matches how existing partition-grain
  windowing is driven, and never widens the derived reach clamp.

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences — narrow "The contract lattice is specified
and unimplemented" to say the `frozen_horizon` declaration, its validation, and its write clamp are
implemented; the late-arrival probe, `deferral`, the parameterised oracle, and the explain surface
remain. `docs/specs/diagnostics.md` §Known divergences — add a line stating
`ContractFrozenHorizonInvalid` now has a `DiagnosticCode` variant and a live derivation site, while
the other three contract codes remain catalogue-ahead-of-variant. No surface-semantics change:
§"Contract relaxations (`contract:`)" already specifies everything being built.

## Tests (red-green)

- `smelt-core` `crates/smelt-core/tests/contract_frozen_horizon.rs`
  - `frozen_horizon_parses_into_metadata` — `contract: { frozen_horizon: '90 days' }` in model
    frontmatter round-trips into `ModelMetadata::contract` with 90×86400 seconds.
  - `unparseable_frozen_horizon_is_a_metadata_error` — `'90 fortnights'` yields
    `MetadataError::ContractFrozenHorizonInvalid`, never a silent `None`.
  - `deferral_key_is_refused_until_wired` — `contract: { deferral: '6 hours' }` is an unknown-key
    parse error naming `deferral` (fail-loud staging, per the decision above).
- `smelt-logical` unit tests in `src/contract/frozen_horizon.rs`
  - `key_grain_declaration_is_refused` — `validate_frozen_horizon` on a key-grain model returns the
    `ContractFrozenHorizonInvalid` reason naming the grain.
  - `partition_grain_declaration_is_admitted` — returns `Ok`.
  - `clamp_narrows_start_to_end_minus_h` — a 400-day run range with `H = 90 days` floors at
    `end − 90d`.
  - `clamp_never_widens` — a run range shorter than `H` is returned unchanged.
- `smelt-db` `crates/smelt-db/tests/contract_frozen_horizon_diagnostics.rs`
  - `frozen_horizon_on_key_grain_model_raises_diagnostic` — `file_diagnostics()` carries
    `DiagnosticCode::ContractFrozenHorizonInvalid` at the declaration's range.
  - `frozen_horizon_on_partition_grain_model_is_clean` — no diagnostic.
- `smelt-runtime` `crates/smelt-runtime/tests/contract_frozen_horizon_clamp.rs`
  - `declared_horizon_narrows_the_write_range` — a partition-grain model with a wide requested
    range and `frozen_horizon: '30 days'` produces no batch whose `partition_start` precedes
    `end − 30d`.
  - `absent_contract_leaves_the_range_untouched` — the default point is byte-identical to today's
    batch decomposition.

## Tasks

1. Add `ContractConfig { frozen_horizon: Option<DataLatency> }` (`deny_unknown_fields`) to
   `crates/smelt-core/src/config.rs`; add `contract: Option<ContractConfig>` to `ModelMetadata`
   and register `("contract", &[DeclarationKind::Model])` in `crates/smelt-core/src/frontmatter.rs`.
2. Add `MetadataError::ContractFrozenHorizonInvalid { model, why }` in
   `crates/smelt-core/src/metadata.rs`; extend the exhaustive
   `map_metadata_error_to_diagnostic` match in `crates/smelt-db/src/lib.rs`.
3. Create `crates/smelt-logical/src/contract/mod.rs` + `frozen_horizon.rs` (module doc naming the
   single-owner triple and which leg lands when); implement `validate_frozen_horizon(grain, h)` and
   the pure `clamp_write_range(start, end, h_seconds)`; export from `lib.rs`.
4. Wire validation: raise `DiagnosticCode::ContractFrozenHorizonInvalid` (new variant in
   `crates/smelt-db/src/diagnostics_types.rs`) from `check_file_diagnostics`, calling the pure
   validator — no grain re-derivation in `smelt-db`.
5. Wire the clamp: narrow `full_range` in `build_model_plans`
   (`crates/smelt-runtime/src/execute.rs:~3981`) through `clamp_write_range` before
   `compute_incremental_windows_ordered`; log at `info` when the declared horizon narrows the
   requested range (rendering in `explain` is phase 6).
6. Apply the spec delta above; keep `crates/smelt-logical/tests/contract_lattice_spec.rs` green
   (adjust only the assertions that encode "unimplemented", if any).
7. Add a `frozen_horizon:` declaration to one partition-grain model in an existing example
   workspace so `example_diagnostics` covers the clean path on real fixtures.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test contract_frozen_horizon`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-db --test contract_frozen_horizon_diagnostics`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-runtime --test contract_frozen_horizon_clamp`
- Env note from phase 1: export `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH` to
  `~/.local/lib/duckdb` before any cargo invocation.

## Commit message

`feat(contract-lattice): frozen_horizon declaration, validation, and write-eligibility clamp`
