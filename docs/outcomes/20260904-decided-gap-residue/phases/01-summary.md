# Phase 1 summary — `ContractFrozenHorizonInvalid` driving-source posture leg

**Shipped:**
- `validate_frozen_horizon_posture(driving_source, profile)` in
  `crates/smelt-logical/src/contract/frozen_horizon.rs`, exported from
  `smelt-logical`'s `lib.rs`. Refuses a `Some(Mutable)`/`Some(ChangeFeed)`
  driving-source profile, admits `Some(AppendOnly)` and `None` (undeclared).
- Wired into `check_file_diagnostics` (`crates/smelt-db/src/lib.rs`): resolves
  the driving relation from the FROM clause's first entry (same parse
  pattern as `smelt_logical::maintenance::locality::resolve_driving_source`),
  looks it up via the existing `ref_source_info` helper, and accumulates
  `ContractFrozenHorizonInvalid` on refusal.
- `docs/specs/diagnostics.md` catalogue row + derivation-sites bullet, and
  `docs-site/docs/guide/incremental-models.md`'s `frozen_horizon` bullet,
  updated with the new condition (spec-first, before the code).
- Tests: 4 `smelt-logical` unit tests, 3 new `smelt-db`
  `contract_frozen_horizon_diagnostics.rs` integration tests (mutable-source
  refusal, append-only clean, joined-mutable-dimension ignored), 1 LSP e2e
  test (`crates/smelt-lsp/tests/e2e.rs`) asserting the published code slug
  `contract-frozen-horizon-invalid`, 1 `examples/broken` fixture +
  `example_diagnostics.rs` test.
- Fixture: `examples/broken/models/sources/contract_mutable_orders.yml` +
  `examples/broken/models/contract_frozen_horizon_mutable_source.sql`.
- `docs/TODO.md`'s "Frozen-horizon append-only gate" bullet removed.

**Decisions:**
- Undeclared mutation profile is admitted (settled in the plan, not newly
  decided here): the spec refuses on "any other *declared*" profile, so
  `None` has nothing declared to contradict; `SourceMutationProfileViolated`
  already polices the undeclared case at run time.
- The `examples/broken` fixture needed `maintenance.scan_bounds.per_source.
  contract_mutable_orders.allow_full_scan: true` — without it the model also
  tripped `MaintenanceScanUnbounded` (a mutable-snapshot driving source with
  no clock scatters across all output partitions), which broke the existing
  `broken_workspace_maintenance_scan_unbounded` test's "no other file raises
  this code" assertion. Accepting the full scan isolates the fixture to
  exactly the one diagnostic under test.

**For the next planner:**
- Phase 1's own scope is fully closed; nothing deferred out of it.
- Phase 2 (deferral oracle restatement) is next per the outcome's table —
  no discoveries here bear on it.
- Three `MutationProfile` enums now exist across crates
  (`smelt-core::sources`, `smelt-logical::maintenance`,
  `smelt-logical::analysis::input_delta`) with a manual `From` conversion
  between two of them. Not this phase's problem, but worth a note if a
  future outcome ever touches mutation-profile plumbing broadly.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_lattice_spec` — 13 passed.
- `cargo test -p smelt-db --test contract_frozen_horizon_diagnostics` — 5 passed.
- `cargo test -p smelt-lsp` — all passed (incl. new e2e test).
- `cargo test -p smelt-cli --test example_diagnostics` — 120 passed, 1 ignored.
- `cargo test -p smelt-runtime --test statement_parity` — 37 passed.
