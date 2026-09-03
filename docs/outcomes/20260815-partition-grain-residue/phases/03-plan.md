# Phase 3 plan — CTE-only `event_time_column` detection in the outer-visibility check

## Objective

Close audit residue #6 (success criterion 3): a batched model whose outer `FROM` names a CTE that
does not project `event_time_column` must be rejected with
`EventTimeColumnNotVisibleAtOuterSelect` at check time, not fail at execution. Today
`check_event_time_injectable`'s Case 2 only matches a *bare parenthesized* subquery in `FROM`
(`from_text.starts_with('(')`), so the `WITH … SELECT … FROM <cte>` form escapes entirely.

## Spec delta (spec-first — make this edit before the code)

- `docs/specs/incremental_shapes.md` §"The partition grain" → §"Event-time outer-visibility"
  (~line 528): extend the rejection sentence to name the CTE case — a `FROM` naming a CTE whose
  body does not project `event_time_column` is rejected the same way as a bare subquery, resolved
  through a chain of CTEs, and left accepted (conservative) when the body projects a wildcard, the
  CTE is recursive, or the outer `FROM` has more than one table expression.
- Same file §Known Divergences: delete the `**CTE-only \`event_time_column\` references are not yet
  detected**` bullet (~line 1161).
- `docs/specs/diagnostics.md` line 134: widen the `EventTimeColumnNotVisibleAtOuterSelect`
  description from "a subquery" to "a subquery or CTE that does not project the column".

## Tests (red first)

In `crates/smelt-logical/src/rules/rule_diagnostics.rs` unit tests (or an existing sibling test
module for this rule):

1. `cte_not_projecting_event_time_is_rejected` — `WITH recent AS (SELECT user_id, amount FROM
   smelt.orders) SELECT … FROM recent` fires `EventTimeColumnNotVisibleAtOuterSelect` and the
   message names `recent`.
2. `cte_projecting_event_time_is_accepted` — same shape with `event_ts` in the CTE select list →
   no diagnostic.
3. `cte_wildcard_projection_is_accepted` — `WITH recent AS (SELECT * FROM smelt.orders)` →
   conservative accept.
4. `chained_cte_missing_event_time_is_rejected` — `WITH a AS (SELECT user_id FROM smelt.orders),
   b AS (SELECT user_id FROM a) SELECT … FROM b` → rejected, naming the CTE the outer FROM binds.
5. `cte_column_list_alias_is_used_for_projection` — `WITH recent(user_id, event_ts) AS (SELECT a,
   b FROM smelt.orders)` → accepted (the declared column list, not the body's select list, is the
   CTE's projection).
6. `multi_table_from_with_cte_is_not_rejected` — outer `FROM recent JOIN smelt.orders o ON …` →
   conservative accept (the column may come from the other side).
7. `recursive_cte_is_not_rejected` — `WITH RECURSIVE …` → conservative accept, no hang.
8. `plain_table_from_is_unaffected` — `SELECT … FROM smelt.orders` regression guard.
9. `crates/smelt-logical/tests/partition_residue_probes.rs::probe_cte_only_event_time_column` —
   **invert**: assert the diagnostic now *does* fire; update its doc comment to record the residue
   as landed in phase 3.

## Tasks

1. Make the spec + diagnostics-catalogue edits above.
2. Write tests 1–8 red against the current implementation.
3. In `check_event_time_injectable`, after the existing Case 2, add Case 3: build a
   `name → Cte` map from `stmt.with_clause()`; if the outer `FROM` has exactly one `TableRef` and
   no `joins()`, and that ref's `identifier()` matches a CTE name, resolve its projection.
4. Projection resolution for a CTE: prefer `Cte::column_names()` when non-empty; otherwise run
   `is_column_projected_in_sql` over the CTE body SQL (`Cte::query()`, parens stripped). Follow a
   chained reference (body's own single-table `FROM` naming another CTE) with a visited set and a
   small depth cap; return "conservatively projected" for a wildcard, a set-operation body, a
   recursive `WITH`, an unresolvable name, or a cycle/depth-cap hit.
5. Emit `EventTimeColumnNotVisibleAtOuterSelect` (Error) naming the CTE alias and instructing the
   author to add the column to that CTE's select list.
6. Invert the probe (test 9) and update the probes module doc.
7. Sweep `examples/` for batched models with a CTE-shaped outer `FROM` — any new diagnostic is a
   true positive to fix in the example (add the column) or a classifier over-reach to narrow;
   decide per model and record which in the summary.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test partition_residue_probes`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(logical): reject CTE-hidden event_time_column at the outer-visibility check`
