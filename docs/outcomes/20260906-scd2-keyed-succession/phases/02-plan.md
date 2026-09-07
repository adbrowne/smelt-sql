# Phase 2 plan — Keyed-succession classifier leaf

## Objective

Land `classify_keyed_succession` as a pure leaf classifier in
`crates/smelt-logical/src/analysis/succession.rs`, implementing
`docs/specs/model_properties.md` §"Keyed-succession classification" rules 1, 1a, 1b, 2–6,
with a refusal-reason enum that is 1:1 with the eleven analysis-time codes phase 3 will map
to `DiagnosticCode` variants. Invoked only from the composition walk. This is criterion 1
in full, and the seam every later phase reads (`Recognized{…}` is the input to the grain,
the emitters, and the explain rendering).

## Spec delta

None. The spec branch already carries the normative rules; the `not-yet` status cell in
`model_properties.md`'s classifier table row is rewritten by phase 10, not here.

## Tests

Unit tests live in `succession.rs`'s own `#[cfg(test)] mod tests` (helper builds a
`SuccessionContext` for a fixed `append_only`, clocked source `raw.customer_changes` with
`event_time_column = changed_at`, `NOT NULL` on `customer_id`/`changed_at`/`is_deleted`).

Recognition:
- `recognizes_minimal_lead_shape` — `LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at)` → `Recognized`, `lead_cols` populated, `lag_cols`/`delete_flag`/`pre_filter` empty.
- `recognizes_lag_projection` — a `LAG(changed_at)` sibling lands in `lag_cols`.
- `recognizes_scalar_expression_over_lead` — `LEAD(...) IS NULL AS is_current` and a `CASE`/arithmetic over one `LEAD` whose other operands are constants, the clock, or projected row-local columns.
- `recognizes_qualify_not_flag_as_delete_flag` — `QUALIFY NOT is_deleted` → `delete_flag = Some("is_deleted")`.
- `recognizes_pre_window_clamp_as_pre_filter` — a row-local `WHERE changed_at >= …` is carried as `pre_filter`, admission unchanged.
- `bare_negated_flag_pre_filter_carries_advisory` — `WHERE NOT is_deleted` → `Recognized` **plus** the `PreFilterNegatesFlag` advisory; a paired assertion that the verdict is otherwise byte-identical to the same model without the filter (the advisory never changes admission).

Refusals (one test per named reason, each asserting the reason variant, not just non-recognition):
- `refuses_non_succession_window_function` (`SUM(x) OVER (…)`, `ROW_NUMBER()`), `refuses_lead_over_other_column`, `refuses_lead_with_explicit_offset`, `refuses_lead_with_default_argument` → `WindowFunctionNotLead`.
- `refuses_mixed_partition_keys` → `PartitionKeyMismatch`.
- `refuses_nullable_key`, `refuses_nullable_clock`, `refuses_non_strict_clock` (`CAST(changed_at AS DATE)`), `refuses_clock_not_event_time_column`, `refuses_descending_order`, `refuses_second_sort_key` → `OrderNotMonotoneClock`.
- `refuses_unprojected_key`, `refuses_unprojected_clock` → `IdentityNotProjected`.
- `refuses_aggregate_sibling_projection`, `refuses_non_row_local_projected_column` → `RowLocalColumnViolation`.
- `refuses_join_from`, `refuses_cte_from`, `refuses_subquery_from`, `refuses_set_op` → `SingleSourceOnly`.
- `refuses_mutable_source`, `refuses_change_feed_source`, `refuses_undeclared_mutation_profile`, `refuses_unclocked_source` → `DrivingSourceNotAppendOnly`.
- `refuses_non_row_local_pre_filter`, `refuses_nondeterministic_pre_filter` (`now()`), `refuses_second_pre_filter` → `PreFilterNotRowLocal`.
- `refuses_where_over_window_derived_column`, `refuses_qualify_other_shape`, `refuses_qualify_nullable_flag` → `DeleteFilterMisplaced`.
- `refuses_distinct`, `refuses_group_by`, `refuses_having`, `refuses_order_by`, `refuses_limit` → `PatternUnrecognized` naming the clause.

Wiring:
- `walk_invokes_succession_leaf` (in `walk.rs` tests) — the walk entry point returns the same verdict the classifier does for the minimal shape, and refuses a nested/`UNION`-arm succession projection at the outer scope.
- `cargo test -p smelt-logical --test walk_coverage` stays green with the new file in `SCANNED_DIRS`.

## Tasks

1. Add `crates/smelt-logical/src/analysis/succession.rs`; declare `pub mod succession;` in `analysis/mod.rs`.
2. Define `SuccessionVerdict::{Recognized{source, pre_filter, key_cols, clock_col, lead_cols, lag_cols, delete_flag, advisories}, NotSuccession{reason}}` and `NotSuccessionReason` (eleven variants, each carrying a message string) — one variant per analysis-time code, `PreFilterNegatesFlag` as an `advisories` entry rather than a refusal.
3. Define `SuccessionContext` — the driving-source facts the rules need: `input_delta::SourceShape` (append-only + clock), the source's declared `event_time_column`, its `NOT NULL` column set (from `SourceColumn.nullable`), and a `source_bounds::BoundContext` for the clock trace.
4. Implement rule 1 (single `append_only`, clocked source `FROM`; refuse join/CTE/subquery/set-op) over the walk's `SelectNode`/`InputItem`, not raw text.
5. Implement rule 1a (single deterministic row-local pre-filter → `pre_filter`; bare negated boolean → advisory) reusing `analysis::monotonicity::classify_function_determinism` and `expr_util::split_top_level_conjuncts`.
6. Implement rule 1b (`DISTINCT`/`GROUP BY`/`HAVING`/`ORDER BY`/`LIMIT` refuse, naming the clause).
7. Implement rule 2 (every `OVER (…)` is `LEAD`/`LAG` of the clock at default offset with no default argument, or one scalar expression over exactly one such call whose other operands are constants, the clock, or projected row-local columns); collect `lead_cols`/`lag_cols`.
8. Implement rule 3 (one `PARTITION BY` key set, one ascending single-column `ORDER BY`; `key_cols` + `clock_col` proven `NOT NULL`; `trace_event_time` → `Traceable` with `is_strict` **and** `source_column == event_time_column`).
9. Implement rules 4–6 (key/clock projected row-locally; every other projected column row-local with no aggregate or second window; at most one `QUALIFY NOT <NOT NULL row-local boolean column>` → `delete_flag`).
10. Add the walk entry `pub fn model_keyed_succession(tree: &QueryTree, ctx: &SuccessionContext) -> SuccessionVerdict` in `walk.rs`, applying the classifier to the top `Select` node only and refusing `SetOp`/`Unsupported`; no other call site of `classify_keyed_succession` exists.
11. Tag the module doc comment as a **Leaf classifier** citing `architecture.md` §"Property composition walk rule", per the walk_coverage convention.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-logical succession`
- `rg -n 'classify_keyed_succession' crates/ --glob '!*/tests/*'` — call sites are `succession.rs` itself and `walk.rs` only.

## Commit message

`feat(smelt-logical): keyed-succession leaf classifier with per-rule refusal reasons`
