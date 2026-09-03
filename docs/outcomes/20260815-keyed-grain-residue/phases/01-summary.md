# Phase 1 summary — `KeyedRetractableContribution`: classifier, diagnostic, fixture, test

**Shipped:**
- `join_alias_for_source(sql, source) -> Option<String>` and `source_join_cardinality(sql, source,
  ctx) -> Option<Cardinality>` in `crates/smelt-logical/src/analysis/join_shape.rs`, replacing
  `smelt-runtime`'s private `find_join_alias` (`dimension_join_contribution` now calls the moved
  function). Fixed a latent gap while moving it: the original only resolved an explicit `AS alias`
  or a bare-token `identifier()`, both of which are always `None` for an unaliased `smelt.<path>`
  ref (its tokens live inside a nested `SmeltPathRef`/`SmeltPathCall` node) — added the same
  last-segment fallback `meta_eval`/`maintenance::emit` already use for that shape.
- `Refusal::KeyedRetractableContribution { source, columns, why }` in
  `smelt-logical/src/maintenance/mod.rs`, derived in `derive.rs`'s key-grain `NewData` repair-`Err`
  arm: resolves the source's join alias, composes `fan_out` with each fed fold column's
  `combiner_discriminants` via `join_contribution_monotone`, pushes the refusal only for columns
  whose verdict is `Refused` (additive alongside the pre-existing `NoAdmissibleTechnique` +
  `RepairKeysNotDiscoverable`/`RepairSliceUnbounded` refusals).
- Wired through `smelt-db`: `MaintenanceRefusal::KeyedRetractableContribution`,
  `DiagnosticCode::KeyedRetractableContribution` (Error), and the message arm in `lib.rs` steering
  to `refresh: materialized_view` or DAG composition. Added the LSP kebab-case mapping in
  `smelt-lsp/src/backend.rs` (compile-exhaustive match).
- Tests: `crates/smelt-logical/tests/repair_wiring.rs` (3 new: retractable-fires,
  monotone-doesn't-fire, admitted-repair-never-fires), `crates/smelt-logical/src/analysis/
  join_shape.rs` (3 new unit tests for `join_alias_for_source`: aliased, unaliased, not-joined),
  `crates/smelt-db/tests/maintenance_diagnostics.rs::keyed_retractable_contribution_is_an_error_
  diagnostic` (real workspace, real Salsa pipeline).
- Spec: deleted the "`KeyedRetractableContribution` has no implementation (Open Question)" bullet
  from `docs/specs/incremental_shapes.md` §Known Divergences.

**Decisions:**
- Followed the plan's literal composition (`join_contribution_monotone`'s verdict alone decides),
  not a narrower "only decrementing-aggregate" reading — verified against two designed fixtures
  (SUM/no-unique-key fires; MAX/proven-one-to-one doesn't) that this exactly matches "never fires
  on join spelling alone."
- Left `smelt-db/src/queries/maintenance.rs`'s `RepairKeysNotDiscoverable`/`RepairSliceUnbounded`
  arms mapped to `None` (no `DiagnosticCode` yet) — only corrected the stale comment claiming they
  are "not yet produced by any wired derivation" (they are, by `derive_new_data`); adding their own
  diagnostic codes is a different, unclaimed divergence, out of this phase's scope.

**For the next planner:**
- **Real limitation discovered, not fixed here:** `repair::admit_per_group_recompute` always calls
  `derive_affected_keys` with an empty `JoinContext` (`repair.rs` line ~90) and `delta_shape_for_
  source`'s projected columns never include a join's own `ON`-condition columns — so per-group
  repair can *never* admit (`Ok`) for a source reached only through a JOIN, regardless of
  `unique_key`/clock. Every "genuinely enriched, repair might otherwise admit" scenario in this
  phase's tests had to fall back to a non-joined driving-source fixture instead. This blocks a
  fully faithful `admitted_repair_emits_no_retractable_refusal` test and likely also blocks real
  per-group repair for enrichment-join models in production. Worth its own phase: thread a real
  `JoinContext` (built the same way `KeyedRetractableContribution`'s new code does) into
  `admit_per_group_recompute`, and extend `delta_shape_for_source`'s projection to include a
  source's own join-condition columns.
- Success criterion 1 is fully met; the diagnostic fires end-to-end through the real Salsa
  pipeline, not just the pure-function unit tests.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt-check, clippy zero-warnings both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test repair_wiring` — 8/8 pass.
- `cargo test -p smelt-db --test maintenance_diagnostics` — 30/30 pass.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — 33 + 32 pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74/74 pass (no new refusal on
  admitted recipes).
- `cargo test -p smelt-logical --lib join_shape` — 14/14 pass.
- `rg -n 'KeyedRetractableContribution' docs/specs/` — no "no implementation"/"Open Question"
  wording remains.
