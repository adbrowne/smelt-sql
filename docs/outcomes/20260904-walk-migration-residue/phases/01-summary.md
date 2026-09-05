# Phase 1 summary — expression-position subqueries and parenthesised join groups as walk nodes

**Shipped:**
- `TableRef::nested_table_ref()`/`nested_joins()` (`crates/smelt-parser/src/ast.rs`) and
  `SelectList::syntax()` — small parser accessors this phase needed.
- `normalize_table_ref_items` (`walk.rs`) flattens a parenthesised join group
  (`FROM (a JOIN b ON …)`, arbitrarily nested) into its member `InputItem`s instead of
  `Unsupported`.
- `SelectNode.expr_scopes: Vec<ExprScope>` + `PathSeg::ExprScope { kind, index }` +
  `ExprScopeKind { Scalar, Exists, In, Quantified }`: every scalar/`EXISTS`/`IN`/quantified
  subquery in a scope's own select list/`WHERE`/`HAVING`/`QUALIFY`/`ORDER BY` is now a walk node,
  folded into the children tail as `ctes ++ inputs ++ expr_scopes` (`walk_select`).
- `has_unsupported()` covers `expr_scopes`; its stale doc comment (redundantly-parenthesised
  derived table) is corrected — that shape already normalized to `Derived` before this phase
  (parser-level fact, not this phase's work).
- Audited all 11 production `Transfer` impls for tail-safety against the new `expr_scopes`
  children: 8 were already safe (zip-truncation or ignore `children` outright); 3 needed an
  explicit bound to `ctes.len() + inputs.len()` — `TrajectoryTransfer` (footprint.rs),
  `ReachTransfer` (source_bounds.rs), `SkewTransfer` (walk.rs). Comments on each explain why.
- Spec delta: `docs/specs/model_properties.md` §"The composition walk" gained a paragraph
  enumerating what counts as an operator-tree node; §Known Divergences' bullet narrowed to what's
  still true (bound/reach/grain don't yet *consume* expr-scope verdicts — that's phase 2).
- 8 new tests in `crates/smelt-logical/tests/walk_hardening.rs` (red-green; all passed once the
  fixtures matched the parser's actual shapes).

**Decisions:**
- 2026-09-05: a parenthesised join group's own alias (`FROM (a JOIN b) AS g`) is not modelled —
  deferred as a rare shape distinct from every other `InputItem`, not silently guessed at.
- 2026-09-05: `existing_transfer_verdicts_are_unchanged_by_expression_scopes` compares a fixture
  against its expr-scope-free baseline rather than pinning magic literal values — more robust and
  exactly matches the "behaviour-preserving" claim being tested.
- 2026-09-05: `unsupported_expression_scope_body_is_fail_loud` uses a table-function-in-FROM
  nested inside a scalar subquery (not a literal `(VALUES …)`) — the grammar never produces a
  `SUBQUERY` node wrapping `VALUES` at expression position (only in FROM position), so the planned
  VALUES fixture wasn't constructible; the substituted fixture exercises the same
  `has_unsupported` code path via `expr_scopes`.

**For the next planner:**
- Phase 2 (bound/reach and grain consuming expr-scope verdicts) is next; this phase deliberately
  left every transfer's *verdict* unchanged — only structural reachability changed.
- The `expr_scopes` doc comment on `SelectNode` records the full per-impl tail-safety audit result
  — read it before touching any of the 11 `Transfer` impls again, so a new one isn't added without
  re-auditing.
- Nothing found outside this outcome's scope.

**Gates:**
- `cargo test -p smelt-logical --quiet` — 743+ tests, all green (including the 8 new + existing
  `walk_hardening` tests).
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
