# Phase 27c plan — keyless (whole-row `EXCEPT ALL`) staged-candidate realisation

## Objective

Build the whole-row-identity realisation of the staged-candidate conditional write, so a region
whose row identity is `RowIdentity::WholeRow` (no declared `unique_key`, no proven grain key) can
still skip a maintenance write whose applied effect is the identity. Closes the "the whole-row
(`EXCEPT ALL`-both-ways) realisation for a keyless region … remains unbuilt" clause of
`model_transforms.md` §Known Divergences and the matching "the whole-row (keyless)
staged-candidate realisation does not exist" clause in `incremental_models.md` §Known Divergences
"Conditional-maintenance gaps" — success criteria 14/16 (no dead synthesis layer; divergence
bullets retired clause by clause).

## Design calls (do not re-litigate)

- **Suppression granularity is the region, not the row.** Without a row address, a multiset
  difference cannot be deleted with multiplicity in portable SQL. The sound realisation is a
  two-way `EXCEPT ALL` diff over (stored region, staged candidate), materialised once into a
  1-row-max sentinel relation, guarding an otherwise-unchanged region `DELETE`+`INSERT`: diff
  empty ⇒ neither statement touches a row; diff non-empty ⇒ byte-identical to today's
  unconditional region write. Same fixed-`S` bit-equality obligation as the keyed variants, at a
  coarser suppression grain. The spec must say this explicitly rather than implying per-row
  suppression for the keyless case.
- **The sentinel is materialised before the `DELETE`, never re-evaluated after it.** An `EXISTS`
  subquery evaluated after the `DELETE` has already mutated the target is order-dependent
  reasoning; a temp sentinel relation is not.
- **No observed delta is recorded on this path.** The observed-delta table is keyed by the row
  identity's key columns; a keyless write has none. It records nothing and says so, rather than
  synthesising a fake key.
- **Keyed identity never reaches this emitter.** A `RowIdentity::Key` cell keeps the existing
  keyed staged-candidate/MERGE route; this is a keyless-only fallback, not a preference.

## Spec delta (comes first)

- `docs/specs/model_transforms.md`
  - §"Change-suppressed MERGE and the staged-candidate conditional DELETE+INSERT": state the
    whole-row realisation's shape and its region-grained suppression (all-or-nothing per region,
    guarded by the two-way `EXCEPT ALL` diff), and that it records no observed delta.
  - §Known Divergences: delete the "the whole-row (`EXCEPT ALL`-both-ways) realisation for a
    keyless region" clause from the staged-candidate bullet, leaving the `write:` pin (27d) and
    `smelt-runtime::cumulative` wiring clauses intact.
  - The §"Change-suppressed …" table row's coverage cell ("*partial* (keyed identity only)")
    becomes the keyed+whole-row statement.
- `docs/specs/incremental_models.md` §Known Divergences "Conditional-maintenance gaps": delete
  "the whole-row (keyless) staged-candidate realisation does not exist".

## Tests (red → green)

**`crates/smelt-logical/src/maintenance/emit.rs`** (new `#[cfg(test)] mod
staged_candidate_keyless_tests`)
1. `keyless_group_stages_diffs_and_guards_both_write_legs` — the emitted group is transactional
   and its statements are, in order: stage-shape `CREATE`, candidate `INSERT`, sentinel `CREATE`
   carrying both `EXCEPT ALL` directions, guarded `DELETE`, guarded `INSERT`, two `DROP`s.
2. `keyless_region_predicate_bounds_both_the_diff_and_the_delete` — with a region predicate, the
   stored side of the diff and the `DELETE` both carry it; without one, neither does.
3. `keyless_emitter_needs_no_key_but_refuses_an_empty_candidate_select` — panic contract.

**`crates/smelt-logical/src/maintenance/choice.rs`**
4. `whole_row_identity_admits_keyless_staged_suppression_when_every_column_is_comparable`
5. `whole_row_identity_with_an_incomparable_column_refuses_keyless_staged_suppression` — `why`
   names the column.
6. `key_identity_never_resolves_the_keyless_mechanism` — a proven key falls through to the keyed
   resolver, never here.

**`crates/smelt-runtime/tests/statement_parity.rs`**
7. `staged_candidate_keyless_statements_come_from_the_emitter` — executed-vs-emitted byte-identity
   for the new family, per the maintenance-plan purity gate.

**`crates/smelt-runtime/tests/repair_lowering.rs`** (or the nearest live-DuckDB maintenance test)
8. `keyless_recompute_skips_the_write_when_the_candidate_matches_stored_state` — against a real
   DuckDB backend: unchanged candidate ⇒ zero rows deleted/inserted (assert via a rowid/`ctid`-free
   observable, e.g. a `BEFORE`-snapshot equality plus statement-level row counts).
9. `keyless_recompute_rewrites_the_region_when_the_candidate_differs` — changed candidate ⇒ stored
   state equals the candidate afterwards (the equivalence leg).

## Tasks

1. Land the spec delta above (both files) before touching code.
2. Add `emit::emit_staged_candidate_conditional_keyless(table, staged_relation, sentinel_relation,
   region_predicate: Option<&TargetSlicePredicate>, candidate_select, dialect) -> StatementGroup`
   with the 7-statement transactional shape and the doc comment stating the region-grained
   suppression contract.
3. Add `choice::resolve_keyless_staged_suppression(output_columns, comparability, row_identity)
   -> WriteSuppression` — `Suppressed { compared_columns: output_columns }` only when the identity
   is `WholeRow` and every output column carries `Comparability::Comparable` (absence = refusal,
   fail-closed); `Unconditional { why }` otherwise, naming the offending column or the key.
4. Add `MembershipRecomputeWrite::StagedKeyless { compared_columns }` in
   `smelt-runtime/src/maintenance_driver.rs`, returned from `resolve_live_membership_recompute_cell`
   on the `Technique::DeleteInsert` arm where `resolve_write_suppression` refused *solely* because
   the identity is `WholeRow` and step 3's resolver then admits.
5. Add `execute_staged_keyless_recompute(...)` mirroring `execute_staged_membership_recompute`
   minus the observed-delta leg, and dispatch the new variant from its caller.
6. Wire tests 1–9; run the full sweep (phase 25's summary: a refusal/dispatch change breaks
   non-regression tests outside the phase's own file list — run `cargo test --workspace`, not just
   the listed suites, before declaring green).
7. Delete the two divergence clauses only once the tests are green.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance`
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test --workspace` (full sweep, per phase 25's note)

## Commit message

`feat(maintenance): realise the keyless whole-row staged-candidate conditional write`
