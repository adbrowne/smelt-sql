# Phase 1 summary — spec delta: single normative statement of ledger residency

**Shipped:**
- `docs/specs/state.md`: `last_reviewed` bumped to 2026-09-04; Overview gained the
  cross-reference-discipline sentence; all four Known Divergences bullets now name
  `docs/outcomes/20260904-state-residency/outcome.md` (with criterion numbers) as the tracking
  artifact.
- `docs/specs/run_state.md`: dropped `reconciliation.json` from the `.smelt/` layout tree, the
  locking sentence, the atomic-write sentence, and the Fixed-layout invariant; legacy-migration
  bullet now states a legacy root-level `reconciliation.json` is not migrated; §"Relationship to
  the reconciliation ledger" states the ledger's engine-resident end-state without the inline
  divergence prose; new Known Divergences bullet names the shipped `.smelt/`-resident store;
  §References marks `reconciliation.rs` as the file the move deletes.
- `docs/specs/incremental_models.md`: added an outcome link to the DuckDB-only-ledger
  divergence bullet and a new one-line ledger-residency divergence bullet, both pointing at this
  outcome.
- `docs/specs/incremental_shapes.md`: §"The transactional frontier write (merge ledger)" now
  states the end-state (a recorded `MaintenanceStateDowngraded` downgrade on a ledger-less
  backend, both grades) instead of the re-run-tolerant-only skip-and-report; the matching Known
  Divergences bullet and the `RunReporter::state_structure_unavailable` §References note now
  describe that reporter hook as today's stand-in, not the specified behaviour.
- `docs/specs/diagnostics.md`: new `### State residency` catalogue section (after `### Contract
  lattice`) with `MaintenanceStateDowngraded` (Warning) and `DeclaredContractRequiresState`
  (Error); new Known Divergences bullet noting both are catalogue-ahead-of-variant.

**Decisions:**
- No spec reshaped its structure — per the plan, `state.md` §Surface/§Semantics already stated
  the doctrine normatively (written that way from the outset); this phase was de-restatement in
  the three consuming specs plus new diagnostics-catalogue rows, not a fresh normative draft.
- Left `incremental_models.md` lines 1389-1393 untouched, confirming the plan's assessment that
  it already defers correctly to `state.md`.

**For the next planner:**
- Both `rg` sweeps (task 6: `reconciliation\.json` sites; task 7: `Phase [A-Z0-9]` timeless-oracle
  check) came back exactly as the plan predicted — no follow-up cleanup needed from this phase.
- Phase 2 (engine-resident reconciliation ledger on DuckDB) is unblocked: the spec now states the
  end-state ledger DDL/DML lives in `smelt-logical`'s maintenance layer, transactional with the
  write, and `crates/smelt-state/src/reconciliation.rs` is the file it deletes.
- The two new diagnostics-catalogue rows are catalogue-ahead-of-variant by design (matches the
  existing `Maintenance*`/contract-lattice posture); phases 4-5 land the `DiagnosticCode`
  variants and the derivation/refusal sites that raise them.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full `cargo test`,
  example_diagnostics all green).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed (the coverage gate
  stayed green with the two new catalogue-ahead-of-variant rows, as expected).
- `rg -n 'reconciliation\.json' docs/specs/` — 3 hits, all in `run_state.md`/`state.md`
  §Known-Divergences-class prose (the legacy-migration clause and the two tracking bullets); no
  spec body outside those asserts `.smelt/` residency as normative.
- `rg -n 'Phase [A-Z0-9]' docs/specs/{state,run_state,incremental_models,incremental_shapes,diagnostics}.md`
  — 1 pre-existing hit (`diagnostics.md`'s own statement of the timeless-oracle rule, not a
  violation); no new hits introduced.
