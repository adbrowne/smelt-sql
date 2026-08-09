# Phase 3 plan — Per-group recompute technique: derivation, admission, emitter

## Objective

Land the per-group recompute technique as pure `smelt-logical` machinery: a
`Technique::PerGroupRecompute` variant, a fail-closed admission function that discharges the
three repair obligations (grain, bounded slice, affected keys), a `PlanCell`-producing
derivation, and a pure emitter for the targeted delete+recompute-insert. Advances success
criteria 1, 2 and 6. Routing the existing retraction/reprocessing refusal sites into this
derivation — and the runtime lowering that executes it — is phase 5; this phase leaves the
derivation callable and unit-proven, with no live cell yet.

## Spec delta

`docs/specs/model_properties.md` §"Affected-key discovery" — one sentence pinning the corner
phase 2 flagged: a grain column with no dependency on the delta's own source contributes **no
key requirement**, and when *every* grain column is independent the delta names no finite key
set — the verdict is `NotDiscoverable`, never an unconstrained (whole-table) key set, because
the repair family never widens to a whole-table repair (`incremental_models.md` §"The repair
family"). No user-visible surface change beyond this clarification.

## Tests

Red-green, all in `smelt-logical` (unit tests beside the code; integration in
`crates/smelt-logical/tests/repair_cell.rs`):

1. `repair_admits_keyed_non_invertible_mutation` — keyed model, `MAX(...)` over a mutable
   source with declared `unique_key` and a derivable slice → `RepairAdmission::Admitted`
   carrying the group key and the slice bound.
2. `repair_cell_lands_in_column_merge_corner` — the derived `PlanCell` has
   `corner: Corner::ColumnMerge`, `technique: Technique::PerGroupRecompute`, and carries the
   affected-key columns plus a `ScanClamp` for the changed source.
3. `repair_refuses_when_affected_keys_not_discoverable` — a delta shape `derive_affected_keys`
   cannot resolve → `Refusal::RepairKeysNotDiscoverable` naming the source and the reason.
4. `repair_refuses_when_grain_not_derivable` — no declared `unique_key` and a fan-out join →
   same refusal, reason names the missing grain (obligation 6).
5. `repair_refuses_when_slice_unbounded` — grain and keys resolve but no reach/key-temporal-
   locality route bounds the per-group read → `Refusal::RepairSliceUnbounded`.
6. `repair_refuses_when_every_grain_column_independent_of_delta_source` — the spec-delta
   corner: refuses rather than admitting an unconstrained key set.
7. `repair_over_approximated_keys_are_admitted` — a key superset (extra keys) admits; the
   admission records that it is an over-approximation.
8. `emit_per_group_recompute_deletes_affected_keys_and_inserts_slice_recompute` — the emitted
   group stages the candidate recompute, deletes stored rows for the affected key set (so a
   group that vanished is removed), inserts the staged rows, drops the stage; transactional.
9. `emit_per_group_recompute_is_key_restricted` — every write statement is predicated on the
   affected-key relation; no statement touches the table unrestricted.
10. `emit_per_group_recompute_repeats_identically` — emitting twice yields identical text
    (the Idempotent grading's textual precondition).

Gate-level: `cargo test -p smelt-logical --test walk_coverage` must stay green with no new
raw-text scans (admission reads walk-backed proofs only).

## Tasks

1. Spec delta above (spec-first), one sentence + no other prose churn.
2. Add `Technique::PerGroupRecompute` to `maintenance::mod.rs` with a doc comment placing it in
   the `ColumnMerge` corner and citing §"The repair family".
3. Add `Refusal::RepairKeysNotDiscoverable { source, why }` and
   `Refusal::RepairSliceUnbounded { source, why }`, mapped to the spec's
   `MaintenanceRepairKeysNotDiscoverable` / `MaintenanceRepairSliceUnbounded` codes.
4. New `crates/smelt-logical/src/maintenance/repair.rs`: `RepairAdmission` /
   `RepairRefusal` / `AdmittedRepair { keys, slice, over_approximated }` and
   `admit_per_group_recompute(...)` — obligation 6 via the walk's grain (reuse
   `derive::row_identity_with_context`'s precedence), obligation 4 via the existing reach /
   `locality` route used by `project_source_link`, obligation 7 via
   `analysis::affected_keys::derive_affected_keys`. Fail-closed on each; refusal names the
   failing obligation.
5. `derive_repair_cell(inputs, loc, source, group, …) -> Result<PlanCell, Refusal>` in the same
   module — builds the `ColumnMerge` / `PerGroupRecompute` cell with its scan clamp and row
   identity. Not yet called from `derive_mutation` (phase 5).
6. `emit_per_group_recompute(table, staged_relation, key, affected_keys_select,
   candidate_select, dialect) -> StatementGroup` in `emit.rs`, modelled on
   `emit_staged_candidate_conditional_recompute`: stage → insert candidates → delete stored
   rows matching the affected-key relation → insert staged → drop stage, `transactional: true`.
7. Add arms for the new variant to the downstream exhaustive matches (`smelt-cli` explain
   label + `explain` technique parser, `bakeoff` category, `smelt-runtime` diagnostics
   preview). The `smelt-runtime` lowering dispatch gets an explicit fail-loud arm — no cell
   derives this technique yet; phase 5 lands the executable path.
8. Register `repair` in `maintenance/mod.rs`; run `cargo fmt --all`.

## Verification

- `cargo test -p smelt-logical --lib repair`
- `cargo test -p smelt-logical --test repair_cell`
- `cargo test -p smelt-logical --test walk_coverage`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(incremental): per-group recompute — admission, cell derivation, emitter`
