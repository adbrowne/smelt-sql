**Shipped:**
- `crates/smelt-logical/src/contract/retain_departed.rs` (new): the declaration-half triple —
  `validate` (posture admissibility + tombstone-column existence), `classify_key` (the departed-
  key quotient oracle), `emit_departed_key_probe` (the reconcile anti-join probe). Registered in
  `contract/mod.rs`, re-exported as `smelt_logical::validate_retain_departed`.
- `smelt_core::config::RetainDeparted` (untagged `Bool(bool)` / `Tombstone { tombstone: String }`)
  on `ContractConfig.retain_departed`.
- `MetadataError::ContractRetainDepartedInvalid` + frontmatter pre-validation
  (`bad_retain_departed_reason` in `smelt-core/src/metadata.rs`, mirroring
  `classify_contract_data_latency_error`).
- `DiagnosticCode::ContractRetainDepartedInvalid` (catalogued), wired into
  `map_metadata_error_to_diagnostic` and a new admissibility check in `check_file_diagnostics`
  (`smelt-db/src/lib.rs`) that resolves posture/tombstone facts and calls the pure validator.
  LSP wire-code mapping added in `smelt-lsp/src/backend.rs`.
- `EffectiveContract.retain_departed` + `render_label` support (so `smelt explain` will show it
  once threaded through — not done here, matches `frozen_horizon`'s own landing shape).
- Tests: `smelt-logical` unit tests (5, in `retain_departed.rs`) + `mod.rs` render_label test;
  `smelt-core` config parse test + 2 integration tests in `tests/contract_deferral.rs`; new
  `crates/smelt-db/tests/contract_retain_departed_diagnostics.rs` (4 fixture tests).
- `docs/specs/diagnostics.md` — catalogue row + updated 5-code contract-lattice Known
  Divergences paragraph. `docs/specs/incremental_models.md` — narrowed the "Posture-derived key
  departure" bullet to the runtime-only residue.

**Decisions:**
- "Consumes a mutable snapshot" is resolved the same way `deferral`'s cell-level "has a clock"
  check is resolved: scan the model's `smelt.sources.*` refs via `ref_source_info`, no new
  resolution machinery.
- `validate` takes all four facts (grain, mutable-snapshot flag, tombstone column, output
  columns) in one call rather than two separate functions — the plan's tests exercise it as a
  single admissibility gate, posture first, tombstone second.

**For the next planner:**
- Phase 32b (already queued, `pending`) is the runtime half: the default point's anti-join
  delete leg in the snapshot-reconcile write path, suppressed where phase 32's point is
  declared; extend `statement_parity` to cover the new emitter's dispatch; narrow/remove the
  remaining divergence residue.
- `EffectiveContract.retain_departed` is plumbed but nothing yet calls `effective_contract()`
  with a real cell address for this point the way `smelt explain` does for `deferral` — that
  wiring is naturally part of 32b once there's a runtime consequence worth explaining.
- Row 33 (`Override-ladder reach (Open Question)`) is untouched — separate scope.

**Gates:** `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
full `cargo test` workspace, `example_diagnostics`). Also individually: `cargo test -p
smelt-logical --lib contract`/`retain_departed` (46 passed), `cargo test -p smelt-core --lib
retain_departed` + `--test contract_deferral` (8 passed), `cargo test -p smelt-db --test
contract_retain_departed_diagnostics` (4 passed), `cargo test -p smelt-db --test integration
diagnostics_catalogue` (1 passed), `cargo test -p smelt-runtime --test statement_parity` (32
passed, unaffected).
