# Phase 27g plan — runtime dispatch for the keyed-fold `write:` pin

## Objective

Make 27d's pure selection layer live: the keyed-fold write path
(`crates/smelt-runtime/src/cumulative.rs` + `maintenance_driver.rs`'s
`run_windowed_keyed_maintenance`) resolves its write mechanism through
`choice::resolve_keyed_write_mechanism` with the cell's matching `write:` pin, so a
`write: staged_candidate` pin actually executes the merge-less staged-candidate group and a
`write: keyed`/`keyed_conditional` pin on a merge-less backend refuses before any write instead
of silently substituting. Advances the success criterion that every derived maintenance decision
is dispatched (not merely derived), and narrows the `incremental_models.md` Known Divergences
"Conditional-maintenance gaps" bullet.

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences → "Conditional-maintenance gaps": drop the
leading clause "no `write:` pin selects between keyed MERGE and staged-candidate" (now wired);
keep the remaining `supports_fingerprint_sidecar` clause verbatim, re-flowed. §"Per-cell write
addressing" → "Within-family mechanism pins" (landed in 27d) already states the user-visible
semantics — no surface change, so `docs-site/docs/reference/smelt-yml.md` needs no edit; confirm
that during review rather than assuming.

## Tests

Red-green, in this order:

1. `cumulative.rs` unit `write_group_with_no_pin_is_byte_identical_to_the_merge` — regression
   guard: the unpinned path yields a one-statement group whose SQL equals
   `build_cumulative_merge_sql`'s current output.
2. `cumulative.rs` unit `staged_candidate_pin_selects_the_staged_candidate_group` — a
   `staged_candidate` pin over a `Suppressed` verdict yields the 5-statement group
   byte-identical to `emit_staged_candidate_conditional` over
   `keyed_fold_candidate_select`'s candidate SQL.
3. `cumulative.rs` unit `staged_candidate_pin_over_an_unconditional_cell_refuses` — the
   `ChoiceRefusal` propagates as an error naming the model and the pin; no MERGE is built.
4. `smelt-db` unit `keyed_fold_write_pin_matches_on_the_driving_source_address` and
   `keyed_fold_write_pin_ignores_a_cell_addressed_at_another_source` — the whole-row keyed cell
   matches a `cells[]` entry by its `on:` address alone (it is a `{*}` cell), sharing the exact
   matching predicate `matching_write_pin` already uses.
5. `smelt-runtime/tests/statement_parity.rs::staged_candidate_keyed_fold_statements_come_from_the_emitter`
   — a real `execute_project` run of a `refresh: keyed` model pinned `write: staged_candidate`:
   captured statements byte-identical to the emitter group, and the resulting table
   `multiset_equal` to a full refresh.
6. `smelt-runtime/tests/statement_parity.rs::keyed_pin_on_a_merge_less_backend_refuses_before_any_write`
   — capturing fake backend with `supports_merge: false` and `write: keyed`: the run bails with
   the pin refusal and issues no write statement.

## Tasks

1. `crates/smelt-db/src/queries/maintenance.rs`: extract `matching_write_pin`'s
   trigger-address/group predicate into one private helper, and add
   `pub fn keyed_fold_write_pin(metadata: &ModelMetadata, driving_source: &str) -> Option<String>`
   over it — the keyed fold's cell is whole-row, so it matches on the `on:` address alone
   (document that, and that it never re-derives admission).
2. `crates/smelt-runtime/src/maintenance_driver.rs`: add
   `WindowedKeyedRule::write_group(&self, schema, table, delta_sql, slice, mechanism:
   &KeyedWriteMechanism, dialect) -> StatementGroup`, defaulting to a one-statement group wrapping
   `merge_sql` for `KeyedWriteMechanism::Merge`. Keep `merge_sql` as the `Merge` arm's emitter so
   the unpinned path stays byte-identical.
3. `crates/smelt-runtime/src/cumulative.rs`: implement `write_group`'s `StagedCandidate` arm —
   `keyed_fold_candidate_select(schema.table, unique_key, folds, delta_sql, dialect)` fed to
   `emit_staged_candidate_conditional` with a staged relation named off the target table.
4. `run_windowed_keyed_maintenance` gains a `write_pin: Option<&'static WritePattern>` parameter;
   resolve the mechanism **once**, before the step loop, via `resolve_keyed_write_mechanism(
   suppression, backend.capabilities().supports_merge, write_pin)`; `Err(ChoiceRefusal)` and
   `Ok(None)` both `bail!` naming the model and pin, before any backend call.
5. Driver step loop: build `action_group` from `write_group` instead of a bare `action_sql`;
   thread it unchanged into both the ordinary `execute_statement_group` arm and the
   observed-delta `execute_conditional_write_and_record_observed_delta` arm (which already takes a
   group). Error context uses the group's statements joined, not a lost single string.
6. `execute_cumulative_aggregate`: look the pin up via task 1 from `model.metadata` +
   `classification.driving_source.name`, resolve it through `lookup_write_pattern`, pass to the
   driver. `execute_snapshot_reconcile`: same lookup, and resolve the mechanism around its direct
   `build_cumulative_merge_sql` call so a pin refuses there too rather than being ignored.
7. Add the two `statement_parity` legs (tests 5–6) and extend that file's module doc to name the
   staged-candidate keyed-fold family.
8. Apply the spec delta; re-read the "Conditional-maintenance gaps" bullet after editing to
   confirm the surviving clause still reads as a standalone sentence.

## Verification

- `bash .claude/scripts/verify-phase.sh` (must be ALL GREEN)
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test observed_delta`
- `cargo test -p smelt-logical --lib maintenance::choice --lib maintenance::emit`
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics`
- `cargo test -p smelt-cli --test cli_unit cumulative_equivalence`
- `cargo test -p smelt-cli --test maintenance_conformance` (equivalence-invariant gate — the
  unpinned keyed path must be unchanged)

## Commit message

`feat(maintenance): dispatch the keyed-fold write: pin to the staged-candidate mechanism at run time`
