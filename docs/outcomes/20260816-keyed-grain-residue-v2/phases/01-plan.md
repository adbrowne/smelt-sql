# Phase 1 — Snapshot-reconcile anti-join delete leg + conformance key-departure staging

## Objective

Make a snapshot-reconcile run delete keys that have departed the upstream: the reconcile write
becomes a transactional statement group of the existing `MERGE` plus an anti-join `DELETE`,
emitted by a pure emitter in `smelt-logical` and executed through
`Backend::execute_statement_group`. Stage key departure in the conformance suite and drop the
retained-departed exemption from the oracle comparison. Advances success criterion 1 (and is the
precondition for criterion 2's `retain_departed` relaxation, which restores retention only under
declaration).

## Spec delta

None. `incremental_shapes.md` §"Departed keys and deletion" and §"The two run shapes" already
state the deletion rule normatively (landed by the decision track, PR #167). The matching Known
Divergences bullet ("Departed keys are still retained under snapshot-reconcile") also names the
unbuilt `retain_departed` point, so it is retired in phase 9, not here. The **stale doc comments**
that assert retention as intended behaviour must be corrected in this phase:
`crates/smelt-runtime/src/cumulative.rs` `execute_snapshot_reconcile` (lines ~491–503) and
`crates/smelt-runtime/src/cumulative.rs:497`'s "`emit_keyed_fold`'s shape carries no `DELETE`".

## Tests

Red-green, in this order:

1. `smelt-logical/tests/emit_statements.rs::snapshot_reconcile_group_pairs_merge_with_departed_key_delete`
   — the new emitter returns `[MERGE …, DELETE FROM <t> WHERE NOT EXISTS (SELECT 1 FROM (<scan>)
   AS s WHERE t.k = s.k)]` with `transactional: true`, the `MERGE` leg byte-identical to
   `emit_keyed_fold`'s current output.
2. `…::snapshot_reconcile_departed_delete_uses_composite_key_conjunction` — a two-column
   `unique_key` renders an `AND`-joined anti-join predicate (mirrors `emit_keyed_fold`'s own
   composite-key handling).
3. `…::snapshot_reconcile_group_suppressed_variant_keeps_the_delete_leg` — the write-suppressed
   dispatch (`emit_keyed_fold_suppressed`) still carries the same `DELETE` leg; suppression
   elides no-op *updates*, never deletions.
4. `smelt-runtime/tests/technique_lowering.rs::snapshot_reconcile_deletes_departed_key` —
   replaces today's `snapshot_reconcile_merges_whole_source_no_window` retention assertion
   (line ~1591, "must never delete a departed key"): the lowered group's second statement is the
   anti-join `DELETE` over the same scan SELECT the `MERGE` uses.
5. `smelt-runtime/tests/technique_lowering.rs::snapshot_reconcile_statements_come_from_the_emitter`
   — the group the executor hands to the backend is byte-identical to the emitter's output
   (maintenance-plan purity / statement-parity leg for this family).
6. `smelt-cli/tests/maintenance_conformance/gate.rs::snapshot_reconcile_plain_overwrite_settles_after_key_departure`
   — rewrite of `snapshot_reconcile_plain_overwrite_settles_with_retained_departed_keys`: same
   seed/mutate/re-run shape, but compared against the **unadjusted** full-scan oracle, plus an
   explicit assertion that the departed key's row is gone.
7. `smelt-cli/tests/maintenance_conformance/gate.rs::snapshot_reconcile_pool_upholds_end_state_equivalence`
   — generative: for each snapshot-admitted combiner (`PlainOverwrite`, and any other combiner
   `KeyedRecipe::new_snapshot_reconcile` admits), drive a generated mutation schedule
   (insert/update/**delete**) through the real `execute_project` pipeline and assert the
   maintained table equals the full-refresh oracle over the current source after every step,
   with no departed-keys exemption.

## Tasks

1. Add `arb_snapshot_mutation_schedule` to `smelt-maintenance-testkit/src/schedule_gen.rs`: a
   `Vec<SnapshotMutation>` (`Insert { id, val }` / `Update { id, val }` / `Delete { id }`) over
   the pool's existing id space, deterministic-seeded like the other keyed strategies; deletes
   drawn only for ids the schedule has already inserted.
2. Add `emit_snapshot_reconcile` to `smelt-logical/src/maintenance/emit.rs`: takes the same
   arguments as `emit_keyed_fold`/`emit_keyed_fold_suppressed` plus the write-suppression choice,
   returns a `StatementGroup { statements: [merge, delete_departed], transactional: true }`.
   Reuse the anti-join `DELETE` shape already proven by the staged-candidate emitter
   (`emit.rs:785`), with the scan SELECT inlined as the anti-join's right side (both statements
   run inside one transaction, so the two scans see one snapshot). No slice predicate: a
   snapshot-reconcile model reconciles the whole target by construction.
3. Wire `smelt-runtime/src/cumulative.rs::execute_snapshot_reconcile` to build that group and
   execute it via `backend.execute_statement_group(&group)` (the same choke point
   `schema_evolution.rs:268` uses — never hand-issued `BEGIN`/`COMMIT` around `execute_sql`).
   Keep the first-run `create_table_as` arm unchanged.
4. Fix the stale doc comments named under "Spec delta"; cite
   `incremental_shapes.md` §"Departed keys and deletion" from the new emitter and executor.
5. Rewrite the gate test (test 6) and drop its `pre_mutation_snapshot` scaffolding. Leave
   `oracle_modes::keyed_end_state_with_retained_departed_keys` and its unit test in place — it
   becomes the `retain_departed` quotient oracle's basis in phase 2 — but it must no longer be
   referenced by any default-point comparison; add a doc line saying so.
6. Add the generative driver (test 7) in `gate.rs`, reusing `stage_keyed_recipe`,
   `insert/update/delete_row_keyed_snapshot`, `snapshot_table_rows`, and the existing
   `keyed_case_count()` case budget.
7. If a backend without `MERGE` (Spark/Parquet) reaches this path, keep today's behaviour
   (that route is already refused upstream) — do not add a new capability branch here.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -30`
- `cargo test -p smelt-logical --test walk_coverage --quiet 2>&1 | tail -10`

## Commit message

`feat(keyed): delete departed keys under snapshot-reconcile via a transactional anti-join leg`
