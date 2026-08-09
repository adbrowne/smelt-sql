# Phase 7 plan — runtime routing for the `diff_patch` write pin

## Objective

Make a `cells[].write: diff_patch` pin actually execute: `ChosenTechnique::DiffPatch`
resolved over a live repair cell lowers to `emit_diff_patch`, and the statements a real
`execute_project` run sends are byte-identical to a direct emitter call. Closes the second
half of success criterion 3 (the executed-vs-emitted `statement_parity` leg) and keeps
criterion 6 green.

## Decisions this phase makes

- **Routed shape is `DiffPatch { recompute: PerGroupRecompute }` only.** That is the one
  recompute whose delete leg `resolve_cell_choice` grants `DeleteLeg::Complete`, and whose
  candidate/slice builders already exist (phase 6). `DiffPatch { recompute: DeleteInsert }`
  (region recompute, delete leg `Omitted`) has no lowering: it **fails loud by name** rather
  than silently falling through to the default write, per fail-loud discipline.
- **No second resolver.** `resolve_live_per_group_recompute_cell` today `continue`s when
  `chosen != Admitted(PerGroupRecompute)`; instead it returns the *write mode* alongside the
  cell. A `diff_patch` write over a repair cell reads the same affected-key set, the same
  candidate select, the same key — only the write leg differs, so a sibling resolver would be
  a near-verbatim copy of a 90-line scan.
- **The emitter's slice restriction is a caller-composed predicate, not a partition region.**
  `emit_diff_patch`'s `partition_col: &str, region: &Region` pair cannot express the repair
  family's slice (a keyed aggregate output has no partition column at all), which is exactly
  the slice the only routable recompute produces. The pair collapses to one
  `slice_predicate: &str` (already `{table}`-qualified, "callers resolve strings, emitters
  assemble"); a region-partitioned caller passes `region.predicate(Some(table), col)` verbatim.
  No shipped statement changes — nothing routed to this emitter before today.

## Spec delta (implement step makes these, first)

`docs/specs/incremental_models.md`:
- §"`diff_patch` — compute, diff, write only the difference": one sentence stating the slice a
  `diff_patch` write restricts to is the *candidate's own* slice — the affected-key set for a
  per-group recompute, a partition region for a windowed one — so the pattern is not tied to a
  partition axis.
- §Known Divergences → "The `diff_patch` write pattern derives but does not yet execute":
  narrow to the still-unrouted case only (a `diff_patch` pin whose underlying recompute is the
  region `DeleteInsert` default refuses at runtime rather than executing a diff), and drop the
  "no executed-vs-emitted parity leg is possible" clause.

## Tests (red first)

1. `smelt-logical` `tests/diff_patch.rs::emit_diff_patch_restricts_both_delete_legs_to_the_caller_slice_predicate`
   — new signature; the update leg and the delete-departed leg each carry the caller's
   predicate verbatim (existing phase-4 emitter tests adapt to the signature).
2. `smelt-runtime` `tests/repair_lowering.rs::diff_patch_pin_over_a_repair_cell_resolves_a_diff_patch_write`
   — resolver returns the diff-patch write mode with non-empty `compared_columns` and
   `DeleteLeg::Complete`.
3. `…::diff_patch_pin_over_a_non_repair_recompute_fails_loud` — a `diff_patch` pin whose
   resolved recompute is not `PerGroupRecompute` errors by name; never a silent default write.
4. `…::repair_slice_predicate_is_the_affected_key_set` — pure builder: an `EXISTS` over the
   clamped affected-key read, `{table}`-qualified on every key column.
5. `…::diff_patch_execution_sends_the_emitted_group` (DuckDB, staged fixture like phase 6's) —
   `execute_diff_patch` executes exactly the emitter's group and leaves the target
   multiset-equal to a full refresh.
6. `smelt-runtime` `tests/statement_parity.rs::diff_patch_statements_come_from_the_emitter` —
   phase 6's `customer_max_amount` fixture plus a `maintenance.cells[{on: raw.orders,
   columns: [max_amount], write: diff_patch}]` pin; run 2 records `strategy == "diff_patch"`,
   the executed group is byte-identical to `emit_diff_patch` over the same inputs, contains the
   delete leg, and the target equals the full-refresh oracle.

## Tasks

1. Make the two spec edits above.
2. `smelt-logical/src/maintenance/emit.rs`: replace `emit_diff_patch`'s `partition_col`/`region`
   parameters with `slice_predicate: &str`; update its doc comment's steps 3/4 rationale and the
   phase-4 unit tests.
3. `smelt-runtime/src/maintenance_driver.rs`: introduce `RepairWrite`
   (`TargetedDeleteInsert` | `DiffPatch { compared_columns, delete_leg }`) and return it (with
   source/cell/key/slice) from `resolve_live_per_group_recompute_cell`; accept
   `ChosenTechnique::DiffPatch { recompute: PerGroupRecompute, delete_leg }` by calling
   `diff_patch::admit_diff_patch(group_columns, comparability, &cell.row_identity, leg→Result)`
   with the comparability derivation the sibling resolvers already use
   (`model_property_vector` + `column_comparability_with_contract`); bail by name for any other
   `DiffPatch` recompute.
4. Same file: add `diff_patch_staged_relation(table)` (`__smelt_diff_patch_{table}` — a distinct
   prefix so a parity test can name the group), `repair_slice_predicate(table, key,
   affected_keys_select)`, and `execute_diff_patch` (emitter → `retry_backend_call` →
   `execute_statement_group`, same shape as `execute_per_group_recompute`).
5. `smelt-runtime/src/execute.rs`: in the keyed window-forward branch, dispatch on the resolved
   `RepairWrite` after the shared affected-key/candidate builders; strategy label `diff_patch`.
6. Write tests 1–6 red, then green.

Explicitly **not** this phase: `smelt explain` rendering of a diff-patch write (phase 9), and a
conformance recipe exercising it (phase 8).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test diff_patch --test walk_coverage`
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity --test technique_lowering --test diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain`

## Commit message

`feat(incremental): route the diff_patch write pin to its emitter at runtime`
