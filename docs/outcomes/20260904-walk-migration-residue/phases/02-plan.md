# Phase 2 — bound/reach and grain consume expression-scope verdicts

## Objective

Retire phase 1's `ReachTransfer` bound and make the grain-side `PropertyTransfer` consume the
`expr_scopes` tail, so a source read only through an expression-position subquery is no longer
invisible to the bound/reach derivation and a projected scalar-subquery column's determinism /
comparability comes from that scope's own verdict rather than the outer classifier scanning across
a node boundary. Advances success criteria 1 (bound/reach/grain consume expr-scope verdicts, with
the inline-equivalence property test) and 5.

## Consumption rule (the design this phase implements)

- **Bound/reach** — an expression-position scope is a *read* of its sources. Its verdict merges
  into the enclosing node exactly like an input's (parallel max-merge, and it participates in the
  sibling-slack computation identically). This makes the verdict equal by construction to the same
  subquery written as a cross-joined derived table, which is what criterion 1's oracle asserts.
- **Grain** — an expr scope contributes **no** key, no output columns, and no fan-out (a filter
  subquery cannot establish a key; a scalar subquery yields one value). It contributes
  (a) `has_set_op_barrier` (a `UNION`-bodied subquery body, matching the inlined derived-table
  form) and (b) per-column determinism/comparability: a select item containing an
  expression-position subquery takes the max of its own syntactic verdict **excluding the subquery
  subtree** and the worst column verdict of the matching `ExprScope` child.
- `expr_determinism` / `expr_comparability` stop descending into `SUBQUERY` nodes — that subtree is
  a walk node now, and descending into it is precisely the cross-node ad-hoc scan the composition
  rule forbids.

## Spec delta (made first, by the implement step)

`docs/specs/model_properties.md`:
- §"The composition walk" — after the operator-tree-node paragraph (line ~319), state the two
  per-property consumption rules above (bound/reach folds an expr scope as a read; grain takes no
  key/fan-out from one but does take the set-op barrier and the per-column determinism /
  comparability verdict).
- §Known Divergences — in the "composition walk is not yet the sole source" bullet, delete the
  "bound/reach and grain do not yet consume their verdicts …" clause; keep the `temporal` /
  anchor-resolution clause, and add that skew and footprint-trajectory still bound the tail
  (this outcome's phase 3).

## Tests (red-green, `crates/smelt-logical/tests/expr_scope_inline_equivalence.rs` unless noted)

1. `scalar_subquery_source_appears_in_bound_map` — a source read only via a select-list scalar
   subquery gets a `BoundResult` (today: absent from the map entirely).
2. `exists_subquery_source_appears_in_bound_map` — same, via `WHERE EXISTS (…)`.
3. `subquery_frame_reach_reaches_model_bound` — a `RANGE BETWEEN INTERVAL '1 day' PRECEDING` inside
   the subquery body yields the corresponding nonzero `before` for that source.
4. `unbounded_construct_in_subquery_is_fail_closed` — `UNBOUNDED PRECEDING` inside a subquery body
   makes the model's verdict `Unbounded`, not silently invisible.
5. `prop_expr_scope_bound_equals_inlined` — proptest: a generated uncorrelated single-column scalar
   subquery, rendered both at expression position and inlined as `FROM t, (…) AS __e`, gives equal
   `derive_model_bounds` maps.
6. `prop_expr_scope_property_vector_equals_inlined` — same generated pair, equal grain,
   determinism and comparability.
7. `scalar_subquery_column_determinism_comes_from_child_verdict` — a `NOW()`-bearing subquery body
   taints the projected column through the child verdict (and the outer classifier no longer
   descends: assert via a body whose function sits only inside the subquery).
8. `exists_filter_does_not_change_grain` — adding an `EXISTS` filter to a `GROUP BY` scope leaves
   grain and `has_fan_out_join` identical.
9. `set_op_bodied_subquery_propagates_barrier` — a `UNION`-bodied scalar subquery sets
   `has_set_op_barrier`, matching the inlined form.

## Tasks

1. Add `range: TextRange` (the subquery node's own range) to `ExprScope` in `walk.rs`, so a select
   item can find the scopes it owns by range containment — the item→scope mapping this phase needs.
2. Make the spec delta above.
3. `ReachTransfer::operator` (`source_bounds.rs`): replace `input_children` with a
   `read_children = &children[sn.ctes.len()..]` slice used in both the merge and the sibling-slack
   loop; delete the phase-1 "not yet a reach contributor" comment and record the consumption rule.
4. `PropertyTransfer` (`walk.rs`): OR the expr-scope children into `input_barrier`; in
   `scope_determinism` / `scope_comparability`, fold in the matching `ExprScope` children's worst
   column verdict per item (matched by range containment).
5. Add a shared descend-but-stop-at-`SUBQUERY` iterator and use it in `expr_determinism` /
   `expr_comparability`; document why (walk-node boundary).
6. Write the tests above (red first), then make them pass.
7. Run the full gate list; if a fixture shows the *inlined* form itself over-narrowing a scan
   (`Bounded{0,0}` for a source only read by an unfiltered aggregate subquery), do **not** paper
   over it — record it in `phases/02-summary.md` as a candidate new phase row, since this phase's
   contract is equality with the inlined form, not fixing that form.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet` (incl. the new test file and `walk_hardening`)
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-planner --quiet` (bound consumers)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`

## Commit message

`feat(logical): bound/reach and grain transfers consume expression-scope verdicts`
