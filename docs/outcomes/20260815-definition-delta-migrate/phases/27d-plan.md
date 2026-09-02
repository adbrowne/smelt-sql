# Phase 27d plan — the `write:` pin selects the keyed-fold write mechanism

## Objective

Make a `maintenance.cells[].write` pin actually choose between the keyed `MERGE` and the
merge-less staged-candidate conditional write within the `Technique::KeyedFold` family, in the
pure layer that owns that choice (`smelt-logical`). Today `choice::resolve_keyed_write_mechanism`
takes no pin at all (its own doc comment records the gap) and always prefers `MERGE` when the
backend has one, so `write: staged_candidate` is silently ignored. This phase also lands the
**folded candidate select** that realisation needs — a keyed fold's staged candidate is
`combiner(stored, delta)`, not the raw delta — so the mechanism 27g dispatches to is sound.
Advances the success criterion that every `write:` pin resolves to a real mechanism or a
fail-loud refusal, never a silent substitution.

## Spec delta

`docs/specs/incremental_models.md` §"Per-cell write addressing" → the **User pins** paragraph
(around line 1034). Add the within-family clause: `keyed`/`keyed_conditional` and
`staged_candidate` all select the keyed-fold *technique* but pin different **mechanisms** —
`keyed`/`keyed_conditional` pin the `MERGE` (unavailable on a merge-less backend →
`MaintenanceWritePatternUnavailable`, already the registry capability's answer), and
`staged_candidate` pins the staged conditional `DELETE`+`INSERT` **even on a `MERGE`-capable
backend** (an explicit choice is not a downgrade). A `staged_candidate` pin over a cell whose
write-suppression verdict resolved `Unconditional` refuses `MaintenanceWriteAddressingRefused`
— the staged shape has no unconditional form — never a silent fall-through to `MERGE`. Absent a
pin, `MERGE` stays preferred wherever the backend has it. Do **not** touch the §Known
Divergences bullet at line ~2080; 27g narrows it once the live path dispatches.

## Tests

Unit tests in `crates/smelt-logical/src/maintenance/choice.rs` (`mod tests`), red first:

- `staged_candidate_pin_selects_staged_mechanism_on_a_merge_capable_backend` — pin +
  `backend_supports_merge = true` + `Suppressed` ⇒ `KeyedWriteMechanism::StagedCandidate`, not
  `Merge`.
- `staged_candidate_pin_over_an_unconditional_verdict_refuses` — pin + `Unconditional` ⇒
  `Err(ChoiceRefusal { pinned: PinnedRequest::Write("staged_candidate"), .. })`, on both backend
  capabilities, never `Ok(Merge)`.
- `keyed_conditional_pin_selects_merge_and_refuses_on_a_merge_less_backend` — pin +
  `supports_merge = false` ⇒ `Err(..)` naming the pin (fail-closed second line of defence behind
  the registry's `WriteCapability::Merge` check), pin + `true` ⇒ `Merge`.
- `an_unpinned_cell_resolves_exactly_as_before` — all four (suppression × capability)
  combinations equal today's asserted results, `Ok(Some(..))`/`Ok(None)`.
- `a_pin_outside_the_keyed_fold_family_leaves_the_default_selection` — e.g. `region`: this
  function does not second-guess `resolve_cell_choice`'s own refusal; the default applies.

Unit tests in `crates/smelt-logical/src/maintenance/emit.rs` (`mod tests`):

- `keyed_fold_candidate_select_folds_stored_state_against_the_delta` — the emitted select
  carries `<combiner>(target.c, delta.c) AS c` for each fold column over a `LEFT JOIN` of the
  delta to the target on the key, so a matched key's candidate row is the folded value.
- `keyed_fold_candidate_select_carries_keys_absent_from_the_target` — a delta-only key appears in
  the candidate with its own (unfolded) values, matching `WHEN NOT MATCHED THEN INSERT`.
- `keyed_fold_candidate_select_feeds_the_staged_emitter_unchanged` — passing it as
  `emit_staged_candidate_conditional`'s `candidate_select` produces a 5-statement transactional
  group whose compare arm names exactly the compared columns.

## Tasks

1. Add `emit::keyed_fold_candidate_select(table, key, folds, delta_sql, dialect) -> String` in
   `crates/smelt-logical/src/maintenance/emit.rs`, next to `expand_aggregator_column_folds` — the
   post-fold candidate rows for a keyed fold, authored in the emitter layer per the
   statement-emission single-owner rule (no runtime-side SQL authoring).
2. Change `choice::resolve_keyed_write_mechanism` to
   `(suppression, backend_supports_merge, write_pin: Option<&'static super::WritePattern>) ->
   Result<Option<KeyedWriteMechanism>, ChoiceRefusal>`; implement the four pin arms above,
   keeping the no-pin path byte-identical.
3. Replace that function's "does not yet consult a `write:` pin (tracked by a later phase)" doc
   paragraph with the resolved semantics; cross-reference the spec section.
4. Update the two existing call sites in `choice.rs`'s own tests to the new signature.
5. Apply the spec delta above.
6. `docs-site/docs/guide/` — add the two pin names and the `Unconditional`-refusal rule to
   whichever maintenance-overrides page already documents `cells[].write` (grep for `write:`
   under `docs-site/docs/`); one short paragraph, no new page.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance::choice --lib maintenance::emit`
- `cargo test -p smelt-logical --test emit_statements`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` (no behaviour
  change expected — the live path is untouched until 27g)
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics`

## Commit message

`feat(maintenance): let a write: pin select between the keyed MERGE and the staged-candidate mechanism`
