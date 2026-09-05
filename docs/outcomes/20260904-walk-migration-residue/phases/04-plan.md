# Phase 4 plan — the cumulative classifier's whole-SQL scans move onto the walk

## Objective

Retire both whole-SQL text scans in `classify_cumulative` (the `OVER(`/`OVER (`
window-function admission scan and the `NONDETERMINISTIC_FUNCTIONS`
`upper_sql.contains(&pattern)` loop) in favour of walk-composed presence verdicts
whose leaf classifiers read one bounded scope's own parsed region. Remove
`rules/cumulative.rs` from `walk_coverage`'s `KNOWN_NONCOMPLIANT` skip-list and widen
that gate so the non-literal scan form it currently cannot see is caught. Advances
success criteria 2 and 5.

## Spec delta (implement step makes these edits first)

- `docs/specs/incremental_shapes.md` §"Key-grain codes" table:
  - `KeyedForbidsWindowFunctions` row — replace "The outer SELECT uses `OVER (...)`"
    with: **any** SELECT scope of the model (outer body, a CTE, a derived table, or an
    expression-position subquery) uses `OVER (...)`. The wider rule is what the
    implementation has always enforced and is the sound one — a window evaluated over a
    delta-filtered scope is not the window over history. Derived by the composition
    walk, so an `OVER` appearing only inside a string literal or comment no longer fires.
  - `KeyedForbidsNondeterministic` row — add that the match is a *parsed function call*
    at any scope, not a substring: a name that merely ends in a listed one
    (`SNOW(...)` vs `NOW(...)`), or a listed name inside a string literal, no longer fires.
- `docs/specs/model_properties.md` §"The composition walk" — add a consumption-rule
  bullet: the keyed-admission presence verdicts (window-function presence,
  non-deterministic-call presence) compose as parallel OR over the whole children slice
  (`ctes ++ inputs ++ expr_scopes`), each node classified over its own region only.
- `docs/specs/model_properties.md` §Known Divergences — delete the
  "`cumulative.rs`'s whole-SQL window-function admission scan" bullet (lines ~413–417);
  it becomes false in this phase. The other three bullets stay for phase 6.

## Tests (red-green)

New, in `crates/smelt-logical/src/rules/cumulative.rs`'s test module unless noted:

1. `over_inside_a_string_literal_does_not_forbid_windows` — RED today: a keyed model
   projecting `'flagged OVER (x)' AS note` is refused by the substring scan; GREEN after.
2. `window_in_a_cte_still_forbids_windows` — fence: today's wider-than-spec refusal is
   preserved by the walk fold (a `ROW_NUMBER() OVER (...)` inside a CTE still refuses).
3. `window_in_an_expression_position_subquery_forbids_windows` — the phase-1/2 walk nodes
   are covered: an `OVER` inside a scalar/`EXISTS` subquery refuses.
4. `nondeterministic_name_suffix_does_not_forbid_nondeterministic` — RED today:
   `SNOW(x)` trips `contains("NOW(")`; GREEN after (parsed call name, exact).
5. `random_in_a_cte_still_forbids_nondeterministic` — fence for the widened-scope leg.
6. `unparseable_scope_shape_still_refuses_via_flat_enumeration` — a shape
   `QueryTree::from_sql` cannot normalize (`has_unsupported`) still refuses on a window,
   proving the CST flat-enumeration fallback is load-bearing (mirrors
   `model_has_trajectory_column`'s fallback).

In `crates/smelt-logical/tests/walk_coverage.rs`:

7. `cumulative_rs_is_covered_by_the_gate` — asserts `scanned_files` contains
   `crates/smelt-logical/src/rules/cumulative.rs` (criterion 2's grep assertion; RED
   while the skip-list entry stands).
8. `detects_an_unclassified_case_folded_variable_scan` — probe fixture binding
   `let upper = s.to_uppercase();` then `upper.contains(&pattern)` in an unclassified fn
   is flagged; a `Vec`/`BTreeSet` membership `.contains(&x)` in the same fixture is not.

## Tasks

1. Apply the spec delta above.
2. In `analysis/walk.rs`, factor `own_region_text`'s pruning traversal (skip `WITH_CLAUSE`,
   root-direct `SELECT_STMT`, every `SUBQUERY`) into a shared `visit_own_region(select, &mut
   impl FnMut(&SyntaxNode))`; re-express `own_region_text` on it, unchanged in behaviour.
3. Add `pub(crate) fn scope_has_window_function(&SelectStmt) -> bool` and
   `pub(crate) fn scope_nondeterministic_fn(&SelectStmt) -> Option<&'static str>` in
   `walk.rs`, both built on `visit_own_region`, both doc-tagged `Leaf classifier` per the
   walk rule. The second matches parsed function-call names against
   `monotonicity::NONDETERMINISTIC_FUNCTIONS` (reuse `own_function_call_names` /
   `classify_function_determinism` where it fits); if the parser models a bare
   `CURRENT_TIMESTAMP` as something other than a call, cover it explicitly and pin it with
   a test rather than letting the migration silently narrow.
4. Add `pub(crate) fn first_scope_hit<T: Clone>(sql: &str, classify: &dyn Fn(&SelectStmt)
   -> Option<T>) -> Option<T>` in `walk.rs`: a `Transfer` with `Verdict = Option<T>`
   folding parallel-OR (first `Some`) over the whole children slice, `Unsupported` mapped
   to the node's own classification; for `QueryTree::from_sql` returning `None` or a tree
   with `has_unsupported()`, fall back to classifying every `SelectStmt` in the parsed CST
   — the same fallback shape `model_has_trajectory_column` uses. Document it as the shared
   scope-presence entry point.
5. In `classify_cumulative`, replace the `upper_sql.contains("OVER(") || …` branch with
   `first_scope_hit(sql, &|s| scope_has_window_function(s).then_some(()))`, and the
   `NONDETERMINISTIC_FUNCTIONS` loop with
   `first_scope_hit(sql, &|s| scope_nondeterministic_fn(s))`. Delete the now-dead
   `let upper_sql = …` binding and the stale "Known walk-invariant violation" comment
   block above the window rule.
6. In `walk_coverage.rs`: empty `KNOWN_NONCOMPLIANT` (keep the const with a doc note that
   an entry now requires a reviewer sign-off), widen `is_raw_scan_line` to also flag
   `<ident>.contains(` where `<ident>` is bound in the same production source to a
   `.to_uppercase()`/`.to_lowercase()` expression (empirically exactly one site crate-wide
   today — the one task 5 removes), and add tests 7 and 8.
7. Run the gates; if any fence in tests 2/5 forces a narrowing of the fold, record the
   narrowing in the spec text rather than weakening the fence (phase 3's precedent).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-logical --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `rg -n 'contains\("OVER' crates/smelt-logical/src` returns nothing.

## Commit message

`feat(logical): keyed admission scans compose over the walk, not whole-SQL text`
