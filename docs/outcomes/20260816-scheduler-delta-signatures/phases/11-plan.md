# Phase 11 plan — walk fix: `GROUP BY` keys resolve against output aliases

## Objective

`analysis::walk::PropertyTransfer::group_by_output_keys` matches each resolved `GROUP BY` key
against a select item's own **expression text** only, so a scope grouping by a projected *alias*
(`SELECT date_trunc('day', ts) AS d, user_id, … GROUP BY d, user_id` — accepted by both DuckDB
and PostgreSQL) fails the grain proof closed for the **whole** scope rather than for the one
unresolvable column. That hard-refuses grain, row identity, and every downstream key-addressed
admission for an entire family of ordinary grouped models with derived columns. Fixing it serves
criteria 1 and 7: phase 12's conformance recipes may then use alias grouping instead of the
constant-projection workaround phase 2 had to write into the testkit.

## Spec delta (implement step makes this first)

- `docs/specs/model_properties.md` §"Region row identity" — the paragraph naming "the walk's own
  proven grain key (…the same `GROUP BY`/`DISTINCT`-factory key…)" gains one sentence pinning how
  a grouping key resolves to an output column: **by the item's own expression text, by its output
  alias, or by ordinal position**, matching what the engines accept; a key resolving to none of
  the three is not projected on the output relation and fails closed to unkeyed — for that scope,
  as today.
- `docs/specs/incremental_shapes.md` §"Safety checks" — the partition-alignment sentence gains the
  same resolution clause, since `scope_group_by_alignment` shares the defect (a scope grouping by
  the partition column's alias currently reports `NotAligned`, a false negative).

No user-facing docs-site change: this is a refusal that stops happening, not new surface.

## Tests (red-green)

`crates/smelt-logical` unit tests (in `analysis/walk.rs` and `analysis/mod.rs` test modules):

1. `group_by_alias_resolves_to_output_key` — `SELECT date_trunc('day', ts) AS d, user_id,
   COUNT(*) AS c FROM t GROUP BY d, user_id` proves `Grain { keys: [[d, user_id]] }` (today:
   unkeyed). **Red first.**
2. `group_by_expression_text_still_resolves` — regression: grouping by the item's raw expression
   text (`GROUP BY date_trunc('day', ts), user_id`) and by ordinal (`GROUP BY 1, 2`) still prove
   the same key set as today.
3. `group_by_non_projected_key_still_fails_closed` — `GROUP BY region` where `region` is neither
   projected nor aliased ⇒ `Grain::unkeyed()`; the fail-closed leg is untouched.
4. `group_by_alias_match_is_case_insensitive` — `GROUP BY D` against `… AS d` resolves (SQL
   identifiers are case-insensitive; the expression-text leg keeps today's exact comparison).
5. `scope_group_by_alignment_accepts_alias_grouping` — a scope projecting
   `date_trunc('day', ts) AS d` with `GROUP BY d` and `partition_column: d` reports `Aligned`
   (today: `NotAligned`).

`crates/smelt-db` integration (extend an existing maintenance/property test file rather than a new
one if a natural home exists):

6. `alias_grouped_model_proves_row_identity` — a real model whose body groups by a projected alias
   derives `RowIdentity::Key{…}` / a non-unkeyed grain through the full Salsa path, proving the
   fix reaches the consumers and not only the transfer function.

## Tasks

1. Land the two spec sentences (spec-first) before touching code.
2. Add a pure helper beside `resolve_scope_group_by` in `crates/smelt-logical/src/analysis/mod.rs`
   — e.g. `resolve_group_by_key_to_output(items, key) -> Option<String>` — that matches expression
   text exactly, then output alias case-insensitively, returning the item's output name (alias, or
   expression text when unaliased). Single owner for both call sites; doc-comment it as a leaf
   classifier over already-classified items (walk rule).
3. Rewrite `group_by_output_keys` to call the helper; keep the `None` ⇒ unkeyed fail-closed
   contract and update its doc comment to name the three resolution routes.
4. Fix `scope_group_by_alignment` to compare each resolved key through the same helper against the
   partition item's output name, instead of comparing raw key text to the partition expression.
5. Write tests 1–6 red, then green (test 2/3 stay green throughout — they are the regression
   fence).
6. Revisit the two phase-2 workaround sites: `crates/smelt-maintenance-testkit/src/dag.rs`
   (`DagBody::PartitionOverKeyedId`) and
   `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`
   (`stage_chain_project_partition_downstream`). Attempt restoring the honest `GROUP BY {d}, {id}`
   shape. If it now passes, keep it and delete the workaround comments. If it still refuses for a
   *different* reason (phase 2's summary flags `derive_affected_keys` returning every grain column
   into `KeyScope` ⇒ `MaintenanceKeyScopeColumnMissing`), revert to the current shape and rewrite
   both comments to name that real remaining reason — never leave the now-false walk explanation
   standing. Record which branch happened in the phase summary; do **not** widen scope into
   `derive_affected_keys` here.
7. Check whether any golden/snapshot output changes (grain now proven where it was unkeyed) and
   regenerate rather than hand-edit.

## Verification

- `cargo test -p smelt-logical --quiet` (includes the standing `walk_coverage` gate).
- `cargo test -p smelt-db --test maintenance_ledger --test maintenance_signature --quiet`.
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --quiet`.
- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN (this fix widens admission across the
  whole workspace's fixtures, so the full sweep is the real gate).

## Commit message

`fix(logical): resolve GROUP BY keys against output aliases in the grain factory`
