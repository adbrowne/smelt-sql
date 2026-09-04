# Phase 1 — Expression-position subqueries and parenthesised join groups as walk nodes

**Outcome:** `docs/outcomes/20260904-walk-migration-residue/outcome.md`
**Advances:** success criterion 1 (structural half), criterion 5 (no new whole-text scan).

## Objective

Make every relational scope a model contains reachable from the normalized `QueryTree`: the
bodies of expression-position subqueries (scalar, `EXISTS`, `IN`, quantified `ANY`/`ALL`) and the
members of a parenthesised join group (`FROM (a JOIN b ON …)`, today an `Unsupported` FROM item).
This phase is **behaviour-preserving**: the new expression-scope verdicts are folded and appended
as a documented tail of the children slice, and every existing `Transfer` impl is audited so its
verdict is byte-identical to today's. Phase 2 makes bound/reach/grain actually consume them.

## Established facts (probed 2026-09-05, do not re-derive)

- `FROM ((SELECT a FROM t)) AS x` already normalizes to `Derived { alias: "x", … }` — the
  parser's `Subquery::select_stmt` unwraps redundant parens. The `has_unsupported` doc comment at
  `crates/smelt-logical/src/analysis/walk.rs:250-257` naming it as *the* known case is stale.
- `FROM (t1 JOIN t2 ON …)` → `InputItem::Unsupported { reason: "unrecognised FROM construct: …" }`.
  The parser nests it as a `TABLE_REF` child of the outer `TABLE_REF`
  (`parser/select.rs:519-530`), so recursion is available.
- `SELECT (SELECT max(b) FROM u) AS m FROM t`, `… WHERE EXISTS (SELECT … FROM u)` and
  `… WHERE a IN (SELECT … FROM u)` all yield a tree with exactly one leaf (`t`); `u` is invisible.
- Impls that slice children with `sn.inputs.iter().zip(&children[sn.ctes.len()..])`
  (e.g. `PropertyTransfer`, `walk.rs:2172`) truncate and are tail-safe by construction; impls that
  fold `children` wholesale are not. There are 11 production `impl Transfer`
  (`footprint.rs`, `monotonicity.rs`, `source_bounds.rs`, `fingerprint.rs`, `affected_keys.rs`,
  `output_delta.rs`, and `walk.rs`'s `ScopeEnum`, `PartitionGrainAdmission`, `SkewTransfer`,
  `PropertyTransfer`, plus the test-only `Discard`/`LineageProbe`).

## Spec delta (first)

`docs/specs/model_properties.md` §"The composition walk": add one paragraph enumerating what
counts as a node of the operator tree — CTE bodies, set-operation arms, derived tables (including
redundantly-parenthesised ones), the members of a parenthesised join group, and the bodies of
expression-position subqueries — and state that an expression-position scope composes as a scope
of its own whose verdict is available to every transfer function, with the per-property
consumption rules deferred to each property's section. Also strike the now-false
`a redundantly-parenthesized derived table (FROM ((SELECT …)) AS t) falls back to the legacy
whole-text derivation` clause from the §Known Divergences "composition walk is not yet the sole
source" bullet (the rest of that bullet stays until phase 5).

## Tests (red-green)

All in `crates/smelt-logical/tests/walk_hardening.rs` unless noted.

1. `redundantly_parenthesised_derived_table_is_a_derived_node` — `FROM ((SELECT …)) AS x`
   normalizes to `Derived`, and `has_unsupported()` is false (regression pin for the fact above).
2. `parenthesised_join_group_is_not_unsupported` — `FROM (a JOIN b ON …)` yields two `Table`
   inputs in the enclosing scope and `has_unsupported() == false`.
3. `nested_parenthesised_join_group_flattens` — `FROM ((a JOIN b) JOIN c)` yields three inputs.
4. `scalar_subquery_body_is_a_walk_node` — `SELECT (SELECT max(b) FROM u) FROM t`: the tree's
   leaf set is `{t, u}` and `u`'s scope carries `PathSeg::ExprScope { … }` in its `NodeCx.path`.
5. `exists_and_in_subquery_bodies_are_walk_nodes` — same for `EXISTS (…)` and `a IN (SELECT …)`.
6. `unsupported_expression_scope_body_is_fail_loud` — an expression subquery whose body is
   `(VALUES …)` normalizes to `QueryNode::Unsupported`, and `has_unsupported()` is true.
7. `expression_scope_verdicts_are_the_documented_children_tail` — a `LineageProbe`-style transfer
   records the children slice for a scope with 1 CTE, 2 inputs and 1 expression scope and asserts
   the order `ctes ++ inputs ++ expr_scopes`.
8. `existing_transfer_verdicts_are_unchanged_by_expression_scopes` — for a fixture with a scalar
   subquery, `derive_source_bounds`, the grain/FD property vector, the fingerprint and the
   output-delta shape each equal the verdict recorded before this phase (pin the current values
   literally; phase 2 is what changes them).

## Tasks

1. Write the spec delta above.
2. Add `PathSeg::ExprScope { kind: ExprScopeKind, index: usize }` and
   `enum ExprScopeKind { Scalar, Exists, In, Quantified }` to `walk.rs`.
3. Add `SelectNode.expr_scopes: Vec<ExprScope>` (`{ kind, body: QueryNode }`), populated by a new
   `normalize_expr_scopes(select, scope)` that walks the scope's **own** clause subtrees (select
   list, WHERE, HAVING, QUALIFY, ORDER BY) for `SUBQUERY` nodes, without descending into a nested
   `SELECT_STMT` (each nested scope collects its own).
4. Recurse `normalize_table_ref` into a nested `TABLE_REF` child (parenthesised join group),
   returning the flattened member items; give `normalize_from` a flat-mapping shape to absorb them.
5. Extend `has_unsupported` to cover `expr_scopes` bodies and correct its stale doc comment.
6. Fold `expr_scopes` in `walk_select` after the input verdicts, pushing the `ExprScope` path
   segment; document the extended children convention on `OpNode`.
7. Audit all 11 production `Transfer` impls: each either slices the tail off explicitly (documented
   with `sn.expr_scopes.len()` arithmetic where it indexes) or is provably truncation-safe. Record
   the per-impl verdict in a comment block on `SelectNode.expr_scopes`.
8. Run the gates; fix fallout in the `smelt-db`/`smelt-planner` consumers if any surfaces.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -40` (whole crate — the audit's blast radius)
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20`

## Commit message

`feat(logical): normalize expression-position subqueries and parenthesised join groups as walk nodes`
