# Phase 3 summary — Per-group recompute technique: derivation, admission, emitter

**Shipped:**
- `Technique::PerGroupRecompute` (`crates/smelt-logical/src/maintenance/mod.rs`), placed in the
  `ColumnMerge` corner, plus `Refusal::RepairKeysNotDiscoverable`/`RepairSliceUnbounded`.
- `crates/smelt-logical/src/maintenance/repair.rs`: `admit_per_group_recompute` (fail-closed over
  obligations 6/7 via `analysis::affected_keys::derive_affected_keys`, obligation 4 via the
  existing `derive::project_source_link` reach/locality route) and `derive_repair_cell` (builds
  the `ColumnMerge`/`PerGroupRecompute` `PlanCell`). Standalone — not called from
  `derive_maintenance_plan` yet (phase 5's scope).
- `emit::emit_per_group_recompute` (`crates/smelt-logical/src/maintenance/emit.rs`): stage →
  insert candidates → key-restricted `DELETE` over the affected-key relation → key-restricted
  `INSERT` from the stage → drop stage, transactional. Both write statements are predicated on
  the same affected-key relation (no unrestricted table write).
- Spec sentence landed in `docs/specs/model_properties.md` §"Affected-key discovery" resolving
  phase 2's flagged corner, and the fix itself in
  `crates/smelt-logical/src/analysis/affected_keys.rs::derive_affected_keys`: when every grain
  column is independent of the delta's source, the verdict is now `NotDiscoverable`, not an
  unconstrained key set (previously a latent bug — the function returned `Keys{cols}` for every
  grain column regardless of source dependency).
- `crates/smelt-logical/tests/repair_cell.rs` (7 tests, plan's tests 1, 3–7 + cell-shape test 2)
  and `emit.rs`'s `per_group_recompute_tests` module (plan's tests 8–10, 4 tests incl. the
  empty-key panic).
- Downstream exhaustive-match arms for the new `Technique`/`Refusal` variants: `choice.rs`'s
  `technique_requires_row_identity` (true), `bakeoff.rs`'s `admitted_family` (`None`, mirrors
  `DeleteInsert`), `smelt-runtime/diagnostics.rs`'s `ALL_TECHNIQUES`/`build_technique_statements`
  (explicit `Err` — no live statement builder yet), `commands/explain.rs`'s `--technique` parser
  (`per_group_recompute`), and `smelt-db/queries/maintenance.rs`'s `Refusal` diagnostic mapping
  (both new refusals left unmapped, same as `ReachNotDerivable`).

**Decisions:**
- 2026-08-09: `AdmittedRepair::over_approximated` is always `true` when admitted — `AffectedKeys::
  Keys`'s own contract already promises a sound-over-approximation may be present; there is no
  cheap way from inside `admit_per_group_recompute` to distinguish an exact match from a wider
  one, so the field surfaces the inherent contract rather than a computed distinction.
- 2026-08-09: `derive::LocalityInputs`/`SourceLink`/`project_source_link` widened from private to
  `pub` (not `pub(crate)`) so `repair.rs` can reuse the exact same derivation `derive_mutation`
  uses for its own scan clamp — one derivation, not a second copy, per "Maintenance-plan purity".
- 2026-08-09: fixed the affected-key "every grain column independent of source" corner in
  `affected_keys.rs` itself (not just repair.rs) since `derive_affected_keys` is the sole owner of
  that proof and the bug was live there regardless of any repair-family caller.

**For the next planner:**
- Phase 5 (per this outcome's own plan-3 reshape note) owns wiring `derive_repair_cell` into
  `derive_maintenance_plan`/`derive_mutation` and the `smelt-runtime` executable lowering —
  `Technique::PerGroupRecompute` currently reaches no live cell, so `diagnostics.rs`'s preview
  entry for it is always `NotApplicable`/build-`Err` today. That's expected per this phase's own
  scope note, not a regression.
- Not investigated: whether `AffectedKeyContext`'s independently-derived grain (inside
  `derive_affected_keys`) and `row_identity_with_context`'s grain can disagree on a real model
  (both use the same fan-out-gated precedence but are two separate call paths into
  `model_property_vector`) — flagged as a residue, no test currently exercises disagreement.
- `crates/smelt-cli/tests/explain.rs` and `crates/smelt-runtime/tests/diagnostics.rs` had
  hardcoded technique counts (4) that needed bumping to 5 — future technique additions will hit
  the same two sites; worth a search before landing a 6th technique.

**Gates:**
- `cargo test -p smelt-logical --lib repair` — n/a (tests moved to `tests/repair_cell.rs`)
- `cargo test -p smelt-logical --test repair_cell` — 7 passed
- `cargo test -p smelt-logical --lib per_group_recompute` (emit.rs) — 4 passed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed, green (no new raw-text scans)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  test, example_diagnostics)
