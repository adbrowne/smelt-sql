# Phase 5 plan — the merge-less conditional write on Trino, and the column-scoped merge's downgrade

## Objective

Make the staged-relation conditional write actually reachable on Trino and prove it, and prove the
`ColumnScopedMerge` cell's landing state there. Two defects block the first today: every production
staged-relation derivation site hardcodes `StagedRelationResidence::SessionTemporary, atomic = true`
(so Trino would be handed `CREATE TEMP TABLE` and a transactional group, ignoring the capability T3
landed), and four emitters spell their delete leg `DELETE FROM t USING s`, which is not Trino
grammar. Advances criteria 2 (reachable families execute), 3 (unreachable families degrade by name),
5 (statements emitted, never authored).

## Spec delta (first)

`docs/specs/multi_backend.md` §"Column-scoped merge and conditional-write capabilities" — add one
paragraph after the `staged_relation_group_is_atomic` bullet stating (a) that residence and
atomicity are read from `BackendCapabilities` by **every** derivation site, never hardcoded per
caller, and (b) the per-dialect spelling of the staged group's delete legs: `DELETE … USING <staged>`
on DuckDB/Spark/BigQuery, and — since Trino has no `USING` clause on `DELETE` — the correlated
`DELETE FROM <t> WHERE EXISTS (SELECT 1 FROM <staged> WHERE <key join> AND (<suppression>))` form on
Trino, **subject to the live measurement task below** (a capability value comes from the warehouse).
The departed-row delete needs no dialect branch: it is already the separate scoped `DELETE` the
`WHEN NOT MATCHED BY SOURCE`-less lowering names. No `model_transforms.md` edit — its
§"The staged-candidate conditional DELETE+INSERT" already states residence-as-capability and the
three non-atomic recovery obligations.

## Tests (red-green)

Unit / structural (no live tier):
1. `smelt-logical` `emit/staged.rs::trino_changed_row_delete_uses_exists_not_using` — the
   `emit_staged_candidate_conditional` changed-row `DELETE` under `MaintenanceDialect::Trino` carries
   no `USING`; DuckDB's text is byte-unchanged.
2. `smelt-logical` `emit/staged.rs::trino_recompute_variant_keeps_a_separate_departed_delete` — the
   `_recompute` variant under Trino emits both delete legs (changed via the new form, departed
   unchanged) and still leads with the non-atomic reclaim `DROP … IF EXISTS`.
3. `smelt-logical` `emit/recompute.rs::trino_diff_patch_and_per_group_deletes_have_no_using_clause` —
   the other two `DELETE … USING` sites (`emit_per_group_recompute`, `emit_diff_patch`'s update leg)
   take the same branch.
4. `smelt-runtime` `tests/staged_relation_atomicity.rs::every_production_derivation_site_reads_the_capability`
   — a structural gate: no `crates/smelt-runtime/src/**` production line spells
   `StagedRelationResidence::SessionTemporary` or `StagedRelation::session_temporary(`.
5. `smelt-runtime` `tests/staged_relation_atomicity.rs::derivation_sites_yield_target_schema_for_trino_caps`
   — each derivation helper returns `TargetSchema`/non-atomic under `BackendCapabilities::trino_iceberg()`
   and `SessionTemporary`/atomic under `duckdb()`.

Live tier (must `<<PHASE_BLOCKED>>`, never skip green, if the coordinator is unreachable):
6. `smelt-backend-trino` `tests/staged_group_live.rs::merge_clause_probe_measures_the_delete_forms` —
   runs each candidate delete form against the live coordinator; the accepted one is what the emitter
   branch spells, and every refusal's error text is quoted in the summary.
7. `smelt-backend-trino` `tests/staged_group_live.rs::staged_conditional_group_executes_on_trino` — a
   real `TargetSchema` group runs through `execute_statement_group`: changed rows rewritten, unchanged
   rows untouched, departed rows deleted, no staged table left behind.
8. `smelt-cli` `tests/trino_incremental_families.rs::membership_recompute_conditional_write_matches_full_refresh_on_trino`
   — a membership-sensitive model through real `execute_project`, oracle-equal to `--full-refresh`.
9. `smelt-cli` `tests/trino_incremental_families.rs::column_scoped_merge_cell_downgrades_and_matches_full_refresh_on_trino`
   — a `ColumnScopedMerge`-electing model on Trino: `smelt explain --json` shows the cell carrying
   `original: ColumnScopedMerge`, `missing: transactional merge ledger`, and the run equals full refresh.
10. `smelt-runtime` `tests/statement_parity/trino.rs::staged_candidate_conditional_parity_on_trino` —
    the executed statements are byte-identical to a direct emitter call over the same inputs.

## Tasks

1. Bring up the tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`) and run test 6's probe
   first — the measured accepted delete form decides the emitter branch's text.
2. Land the spec delta above with the measured form, not the assumed one.
3. Add one shared private helper in the emit layer (single owner) rendering the changed-row delete for
   a dialect, and route all four `DELETE … USING` sites through it (`staged.rs` ×2, `recompute.rs` ×2).
   Tests 1–3 red first.
4. Thread `BackendCapabilities`-derived residence/atomicity into all six production derivation sites:
   `maintenance_driver/membership/execute.rs` (×3, including the keyless staged + sentinel pair),
   `maintenance_driver/delta_restriction/mod.rs`, `maintenance_driver/repair/execute.rs` (×2, whose
   `repair_staged_relation`/`diff_patch_staged_relation` are `pub` and take a new capability argument),
   and `cumulative.rs`. Tests 4–5 red first.
5. Run test 7 live; if the staged relation resolves outside the model's own schema (the client's
   `X-Trino-Schema` default differs from the target schema), qualify the name for `TargetSchema`
   residence in `StagedRelation::derive`, updating its unit test and the spec sentence in step 2.
6. Stage the two CLI fixtures and land tests 8–9; confirm the explain-JSON downgrade shape against
   `trino_explain_downgrade.rs`'s existing assertions rather than inventing a second shape.
7. Land test 10's parity leg beside phase 4's `delete_insert_parity_on_trino`.
8. Write `phases/05-summary.md`; record every measured refusal text and any residue handed to phase 6.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test staged_relation_atomicity`
- `cargo test -p smelt-cli --test trino_incremental_spec_freshness --test trino_ci_wiring --test state_docs_freshness`
- Live tier: `cargo test -p smelt-backend-trino --test staged_group_live -- --test-threads=1`,
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1`,
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1`; tear down with
  `bash scripts/trino-down.sh`.

## Commit message

`feat(trino): execute the merge-less conditional write over a capability-derived staged relation, and prove the column-scoped merge's downgrade`
