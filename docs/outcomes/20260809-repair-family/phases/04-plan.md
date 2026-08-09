# Phase 4 plan — `diff_patch` write pattern: registry entry, emitter, statement parity

**Objective.** Land `diff_patch` as a real member of the open write-pattern registry with a pure
single-owner emitter, so a reconciliation/idempotent re-run write exists as data rather than
prose. Advances success criterion 3 (registered pattern + pure emitter + parity) and, through the
delete-leg gate, criterion 2 (proof-gated admission, reduced capability stated rather than
silently dropped). Execution routing and the executed-vs-emitted parity leg stay in phase 5.

## Spec delta

None — the pattern's semantics, its three contract facts, and the delete-leg degradation are
already normative in `docs/specs/incremental_models.md` §"The write-pattern set is open (and
partly backend-provided)" → "**`diff_patch` — compute, diff, write only the difference**". This
phase implements that text. If the implementer finds a behaviour it must invent (not derivable
from that paragraph plus §"The repair family"'s slice-completeness premise), stop and flag rather
than silently choosing.

## Tests

Red-green, in `crates/smelt-logical/tests/diff_patch.rs` unless noted.

1. `registry_exposes_diff_patch` — `lookup_write_pattern("diff_patch")` resolves; it requires
   `ContractFact::Identity` and `WriteCapability::Always` (the merge-less staged shape).
2. `diff_patch_absent_without_identity` — `admissible_write_patterns` omits it for an
   identity-free output, present for an identity-bearing one on a `MERGE`-less backend.
3. `admit_requires_identity` — `admit_diff_patch` refuses (named refusal, no partial admission)
   when row identity is `WholeRow`.
4. `admit_requires_comparability_for_update_leg` — an incomparable written column refuses the
   update leg by name rather than emitting an unconditional overwrite.
5. `admit_without_slice_completeness_degrades` — no completeness proof ⇒ admitted with
   `DeleteLeg::Omitted { why }`, never `Complete`, never a silent drop.
6. `admit_with_slice_completeness_keeps_delete_leg` — completeness proven ⇒ `DeleteLeg::Complete`.
7. `emit_diff_patch_stages_then_patches` — statement order is stage → update-leg delete →
   (delete-leg delete) → insert → drop stage, one transactional group; every write statement
   carries the slice predicate.
8. `emit_diff_patch_comparison_is_null_safe` — the differ-detection predicate uses
   `IS DISTINCT FROM` over exactly the compared columns, not `<>`.
9. `emit_diff_patch_omits_delete_leg_when_incomplete` — with `DeleteLeg::Omitted` no anti-join
   `DELETE` appears; with `Complete` exactly one does.
10. `emit_diff_patch_rejects_empty_key` — empty key column list panics with the emitter's
    contract message (mirrors `emit_per_group_recompute`'s empty-key contract).
11. `pin_diff_patch_resolves_to_a_diff_write` (in `choice.rs`'s test module) — a `write: diff_patch`
    pin over a recompute-family cell resolves to the diff-patch choice; over a cell whose admitted
    technique is not a recompute family member it refuses with `ChoiceRefusal`, never a blanket
    delete+insert downgrade.
12. `no_maintenance_statement_authoring_outside_the_emitter` (extend, in
    `crates/smelt-runtime/tests/statement_parity.rs`) — the diff-patch statement shapes are added
    to the structural no-authoring scan over `smelt-backend*/src` and `smelt-runtime/src`.

## Tasks

1. Add the `diff_patch` entry to `WRITE_PATTERN_REGISTRY`
   (`crates/smelt-logical/src/maintenance/mod.rs`) — `required_facts: &[ContractFact::Identity]`,
   `capability: WriteCapability::Always`.
2. Extend `WriteSelection` with a `DiffPatch` variant and add the `selects()` arm (the `_ =>`
   arm is an `unreachable!` registry-bug guard — it must not be reached).
3. New `crates/smelt-logical/src/maintenance/diff_patch.rs`: `DeleteLeg { Complete, Omitted { why } }`,
   `DiffPatchRefusal`, and `admit_diff_patch(...)` — identity for the insert/update legs,
   `resolve_write_suppression`'s existing comparability verdict for the update leg (reuse it; do
   not re-derive comparability), slice completeness for the delete leg only. Fail-closed: any
   unprovable premise refuses or degrades by name.
4. Add `ChosenTechnique::DiffPatch { recompute: Technique, delete_leg: DeleteLeg }` and the
   `WriteSelection::DiffPatch` arm in `resolve_cell_choice`/`admits_write_selection`
   (`choice.rs`), admitting only recompute-family cells (`DeleteInsert`, `PerGroupRecompute`) and
   the no-cell region-recompute case; every other cell refuses. Give the two
   `smelt-runtime/maintenance_driver.rs` `ChosenTechnique` comparison sites whatever explicit
   handling the new variant needs (no live routing yet — phase 5).
5. `emit::emit_diff_patch` in `crates/smelt-logical/src/maintenance/emit.rs`: reuse
   `emit_staged_candidate_conditional`'s staging/comparison helpers rather than copying them;
   emit stage → delete-differing → optional anti-join delete → insert-from-stage → drop stage,
   transactional, every write predicated on the slice.
6. Update the hardcoded admissible-pattern list in
   `crates/smelt-cli/tests/fixtures/explain_show_sql_daily_events_golden.txt` (and any sibling
   assertion the gate surfaces) to include `diff_patch`.
7. Extend the `statement_parity` structural no-authoring scan (test 12). Do **not** add an
   executed-vs-emitted leg here — nothing routes to `diff_patch` until phase 5.

## Verification

- `cargo test -p smelt-logical --test diff_patch`
- `cargo test -p smelt-logical --lib diff_patch` (emit + choice unit modules)
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-logical --test walk_coverage` (no new raw-text scans)
- `bash .claude/scripts/verify-phase.sh`

If the docs-site web-analytics `ln:` line is checked by a gate, regenerate it; otherwise leave the
docs-site page to phase 7 and say so in the summary.

## Commit message

`feat(incremental): diff_patch write pattern — registry entry, admission, pure emitter`
