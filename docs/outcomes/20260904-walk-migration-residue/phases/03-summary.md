# Phase 3 summary — skew and footprint-trajectory consume the expression-scope tail

**Shipped:**
- `SkewTransfer::operator` (`crates/smelt-logical/src/analysis/walk.rs`) folds the *whole*
  children slice (`ctes ++ inputs ++ expr_scopes`) by `Skew::union` — no explicit bound remains.
- `TrajectoryTransfer::operator` (`crates/smelt-logical/src/analysis/footprint.rs`) folds `ctes ++
  inputs` unconditionally and folds an `expr_scopes` child only when its `ExprScope::range` sits
  inside one of the enclosing scope's own select-list items — i.e. only when the subquery's value
  flows into a stored output column.
- `OpNode::expr_scopes` doc comment (`walk.rs`) rewritten: no production transfer opts out of the
  tail any more; it now states each transfer's actual consumption rule instead of "pending phase 3".
- 6 new tests: `skew_from_expr_position_subquery_is_seen`,
  `skew_expr_scope_equals_inlined_derived_table`, `prop_skew_expr_scope_inline_equivalence` (64
  cases), `trajectory_from_expr_position_subquery_is_seen`,
  `trajectory_expr_scope_equals_inlined_derived_table` (all in
  `crates/smelt-logical/tests/expr_scope_inline_equivalence.rs`), and
  `skew_self_exclusion_applies_inside_expr_scope` (`tests/skew_self_exclusion.rs`).
- Spec delta applied: `docs/specs/model_properties.md` §"The composition walk" gained a third
  consumption-rule bullet (skew/trajectory); §Known Divergences' walk-migration bullet no longer
  names skew/trajectory as unmigrated.

**Decisions:**
- Skew folds unconditionally (plan's literal instruction) — it has no join-sibling carve-out and
  its fold direction is purely widening, so there is no unsound case to guard against.
- Trajectory does **not** fold unconditionally, despite the plan's task 3 literally reading
  "`children.iter().any(..)` over the whole slice". Implementing that literally broke the existing
  regression fence `window_inside_a_where_subquery_is_not_a_trajectory_of_the_outer_select`
  (`footprint_reflection.rs`) — a running fold buried in a `WHERE`-clause scalar subquery is not a
  *stored column*, so it must not be counted as a trajectory. Per task 6's own instruction ("decide
  whether the widening is unsound … and record the narrowing in the spec text"), narrowed the fold
  to only the `expr_scopes` whose range is contained in a select-list item's own expression range
  (the same range-containment pattern `scope_determinism`/`scope_comparability` already use), and
  wrote the narrowing into the spec delta rather than weakening the fence. All named regression
  fences (`footprint_reflection`, `tracer_propagation`, `locality_projection`, `since_upstream`,
  plus the whole `smelt-logical` suite) pass unmodified.
- First narrowing attempt filtered by `SelectItemKind::GroupByKey` (mirroring
  `scope_has_running_fold_over_axis`'s own filter) — wrong: a bare scalar-subquery select item
  classifies as `GroupByKey` (no window, no aggregate at the *outer* level), so this excluded the
  exact case criterion 1 targets. Fixed by matching directly against `SelectList::items()` /
  `item.expression()`, same as `scope_determinism` does, with no item-kind filter.

**For the next planner:**
- Every named phase-3 test and regression fence is green; `walk_coverage`, `statement_parity`,
  `maintenance_conformance`, and `verify-phase.sh` all pass.
- Phase 4 (cumulative classifier's `OVER(` check) and phase 5 (declared-RI closure to every
  `JoinContext` route) are untouched — nothing here narrows or widens their scope.
- Nothing found out of scope for this outcome.

**Gates:**
- `cargo test -p smelt-logical --quiet` — pass
- `cargo test -p smelt-logical --test walk_coverage --quiet` — pass (4/4)
- `cargo test -p smelt-runtime --test tracer_propagation --quiet` — pass (6/6)
- `cargo test -p smelt-runtime --test statement_parity --quiet` — pass (37/37)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — pass (78/78)
- `cargo test -p smelt-logical --test locality_projection --quiet` — pass (12/12)
- `cargo test -p smelt-cli --test since_upstream --quiet` — pass (19/19)
- `bash .claude/scripts/verify-phase.sh` — `VERIFY: ALL GREEN`
