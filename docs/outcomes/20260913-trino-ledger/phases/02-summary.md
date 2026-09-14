# Phase 2 summary — spec delta: Trino's realisability posture, stated and gated

**Shipped:**
- `docs/specs/state.md` §"Which dialects realise which structure": new `Trino (Iceberg)` column,
  `**no**` on all five rows; prose extended with the measured autocommit-refusal reason
  (`Catalog only supports writes using autocommit: iceberg`), cited to `phases/01-summary.md`,
  stated as permanent.
- `docs/specs/state.md` §Diagnostics: one sentence naming that `MaintenanceStateDowngraded` is
  the normal outcome on a no-structure backend (Spark, Trino) and `DeclaredContractRequiresState`
  is reserved for state-bearing declarations.
- `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend": new paragraph for
  the `trino` target — no correctness structures, recompute-family downgrade via
  `MaintenanceStateDowngraded`, schema evolution named as a separate axis Iceberg does support.
- `docs/specs/multi_backend.md` §"Parity contract": the "not yet reachable" sentence for Trino
  maintenance replaced with the permanent-absence-plus-downgrade framing, keeping the
  `maintenance_dialect`/`SqlDialect::Trino` tokens a pre-existing test requires and adding a
  forward pointer to phase 4.
- New gate `crates/smelt-logical/tests/state_realisability_docs.rs` (4 tests): parses both spec
  tables/sections and asserts they agree with `realisable_state_structures` and with each other;
  fails on a new undocumented `SqlDialect` variant too.
- Doc-comment reword (no behaviour change) in `crates/smelt-logical/src/maintenance/availability/
  state_structure.rs` and the `has_emitters` table in `crates/smelt-logical/tests/
  maintenance_availability/realisation.rs`: both dropped the "`20260913-trino-ledger` revisits"
  framing in favor of "settled by measurement".

**Decisions:**
- Kept `maintenance_dialect`/`SqlDialect::Trino` literal tokens in the reworded §"Parity contract"
  paragraph because `crates/smelt-cli/tests/trino_emission_spec_freshness.rs` (a pre-existing
  gate from `20260913-trino-emission`) asserts on them; added the permanence/downgrade language
  around them instead of replacing the sentence outright. See outcome.md decision log for detail.
- Did not touch `maintenance_dialect`'s `Err` return for Trino — that is phase 4's job; phase 2
  only corrects what the spec says should happen.

**For the next planner:**
- Phase 4 (wire the absence) has its target behavior now written down in both specs — no new
  design decision needed there, just making `maintenance_dialect` route Trino through the same
  availability resolver every other backend uses instead of returning `Err`.
- `docs/specs/state.md`'s realisability table and `multi_backend.md`'s Trino paragraphs are now
  gated against `realisable_state_structures`; a future dialect addition will fail
  `state_realisability_docs.rs` until its column/paragraph is added, which is the intended effect.

**Gates:**
- `cargo test -p smelt-logical --test state_realisability_docs` — 4 passed
- `cargo test -p smelt-logical --test maintenance_availability` — 29 passed
- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` — 7 passed
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`)
- No baseline file touched.
