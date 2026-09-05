# Phase 3 plan — skew and footprint-trajectory consume the expression-scope tail

## Objective

Retire the last two explicit `ctes ++ inputs` bounds phase 1 left behind: `SkewTransfer`
(`analysis/walk.rs`) and `TrajectoryTransfer` (`analysis/footprint.rs`) fold the `expr_scopes`
children tail like every other transfer. This closes the temporary regression phase 2 recorded
(since `own_region_text` now excludes every `SUBQUERY` subtree, a Form B relation living only in
an expression-position subquery is currently *invisible* to skew, not merely double-counted) and
advances criterion 1 — the walk, not a per-transfer opt-out, is what produces a
composition-relevant verdict for a model reading such a scope.

## Spec delta

`docs/specs/model_properties.md`:

- §"The composition walk" — the "Two properties' consumption rules" block becomes three: add a
  **Partition skew and footprint-trajectory** bullet stating that an expression-position scope
  composes into both exactly as any other child does (skew by `Skew::union` — max-before /
  max-after across scopes, since a Form B relation in *any* scope can push rows into a neighbouring
  partition; trajectory by parallel OR — a running fold along the output axis inside a subquery
  body whose value flows into a stored column still makes the model a trajectory). Note that both
  folds are widening, so unlike bound/reach there is no join-sibling carve-out to make: neither
  transfer has a sibling-slack computation, and both verdicts' conservative direction is *more*
  skew / *more* trajectory. Adjust the sentence in the two existing bullets that promises the
  consumption rule "is specified in that property's own section" only if it now reads wrong.
- §Known Divergences, "The composition walk is not yet the sole source of every property" — delete
  the "but partition skew and footprint-trajectory still bound the walk's children tail to
  `ctes ++ inputs` …" clause; the bullet keeps only its `temporal`/driving-fact and
  chained-band residue (phase 6 deletes the bullet outright).

`docs/specs/model_transforms.md` §"The output window is derived, never assumed" — one sentence only
if the section enumerates which scopes contribute skew; leave untouched otherwise.

## Tests

New cases in `crates/smelt-logical/tests/expr_scope_inline_equivalence.rs` (the phase-2 file):

1. `skew_from_expr_position_subquery_is_seen` — a model whose only Form B partition relation lives
   in an `EXISTS (…)` filter body yields non-zero `Skew`; asserts the phase-2 blindness is closed
   (red before the fold widens).
2. `skew_expr_scope_equals_inlined_derived_table` — the skew verdict for a model with an
   uncorrelated scalar subquery equals the verdict for its cross-joined-derived-table rewrite.
3. `prop_skew_expr_scope_inline_equivalence` — `proptest!` (`with_cases(64)`) over the phase-2
   generator asserting the same equality across generated shapes.
4. `trajectory_from_expr_position_subquery_is_seen` — a running `SUM(…) OVER (ORDER BY <axis>)`
   inside a scalar subquery body makes `reflect_footprint` report `Unbounded` for the enclosing
   model (red today: the bound hides it).
5. `trajectory_expr_scope_equals_inlined_derived_table` — same model vs. its inlined rewrite agree.
6. `skew_self_exclusion_applies_inside_expr_scope` — in `tests/skew_self_exclusion.rs`: a
   self-referential model whose self-edge bound sits inside an expression-position subquery is
   still excluded from the skew anchor (the exclusion resolves per scope via `NodeCx::aliases`, so
   it must hold for the new node too), while a genuine Form B relation in the same subquery body
   survives.

Regression fences to run unmodified (widening a skew fold is exactly what over-widened a real
fixture in phase 2): `crates/smelt-runtime/tests/tracer_propagation.rs`,
`crates/smelt-logical/tests/footprint_reflection.rs`,
`crates/smelt-logical/tests/locality_projection.rs`,
`crates/smelt-cli/tests/since_upstream.rs`.

## Tasks

1. Write tests 1 and 4 first; confirm they fail against today's bounds (red).
2. `SkewTransfer::operator` (`analysis/walk.rs`): fold the whole `children` slice by `Skew::union`;
   delete the "Bounded to ctes ++ inputs" comment and the struct doc-comment paragraph recording
   the temporary blindness, replacing it with the consumption rule.
3. `TrajectoryTransfer::operator` (`analysis/footprint.rs`): `children.iter().any(..)` over the
   whole slice; delete the bound comment.
4. Re-read `walk.rs`'s `OpNode` children-convention doc comment and drop any "every production
   transfer either indexes or explicitly bounds" wording that is now stale — after this phase no
   production transfer opts out of the tail.
5. Green the two red tests; add tests 2, 3, 5, 6.
6. Run the regression fences; if one moves, decide (as phase 2 did for sibling slack) whether the
   widening is unsound at that node and record the narrowing in the spec text rather than
   weakening the test.
7. Apply the spec delta.

## Verification

- `cargo test -p smelt-logical --quiet 2>&1 | tail -40`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-runtime --test tracer_propagation --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(logical): skew and footprint-trajectory transfers consume expression-scope verdicts`
