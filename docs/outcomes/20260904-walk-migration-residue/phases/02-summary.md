# Phase 2 summary — bound/reach and grain consume expression-scope verdicts

**Shipped:**
- `ExprScope.range: TextRange` (`walk.rs`) — the subquery node's own range, letting a select item
  find the scopes it embeds by range containment.
- `ReachTransfer::operator` (`source_bounds.rs`): the merge that builds the per-source map now runs
  over `read_children` (`ctes..`, i.e. inputs ++ expr_scopes) — a source read only via a
  scalar/`EXISTS`/`IN`/quantified subquery is now visible and its own-region reach (including a
  `RANGE BETWEEN`/`UNBOUNDED PRECEDING` frame inside the subquery body) folds in correctly.
- `PropertyTransfer` (`walk.rs`): `input_barrier` ORs in the expr-scope children's
  `has_set_op_barrier`; `scope_determinism`/`scope_comparability` take a select item's own
  syntactic verdict (now computed via `own_function_call_names`, which stops descending at a
  nested `SUBQUERY`) maxed with the worst per-column verdict of any `ExprScope` whose range the
  item's expression contains. Grain/FDs/fan-out are untouched — an expr scope contributes neither.
- `own_region_text` now excludes **every** `SUBQUERY` subtree unconditionally (previously only one
  nested in a `TABLE_REF`) — required to stop double-counting an expression-position subquery's
  own Form A/B text once it became a walk node in its own right (phase 1) while `ReachTransfer`
  still scanned the enclosing scope's whole region text for it.
- Spec delta: `docs/specs/model_properties.md` §"The composition walk" gained the two
  bound/reach and grain consumption-rule paragraphs; §Known Divergences narrowed (only skew and
  footprint-trajectory remain unconsuming, phase 3's job).
- 10 new tests in `crates/smelt-logical/tests/expr_scope_inline_equivalence.rs`, including two
  `proptest!` cases (`with_cases(64)`) asserting bound/reach and property-vector equality between
  an expr-position scalar subquery and its uncorrelated cross-joined-derived-table rewrite.

**Decisions:**
- 2026-09-05: **deviated from the plan's literal "participates in the sibling-slack computation
  identically" clause.** `ReachTransfer` merges expr-scope children into the per-source map
  (`read_children`, fixing criterion 1's headline visibility gap) but excludes them from the
  join-sibling slack loop (kept on a separate `join_siblings` slice bounded to `ctes..+inputs`).
  Literal sibling-slack inclusion made an unrelated `bronze_events` FROM input inherit a 14-day
  forward margin from a wholly separate `conversions` scalar subquery in a real fixture
  (`crates/smelt-runtime/tests/tracer_propagation.rs`) — caught by the mandatory workspace
  `cargo test` gate, not by this phase's own (necessarily narrower) proptest, since sibling-slack
  only fires when a real FROM join or a nonzero own-region margin exists and my uncorrelated
  generator never produced one. Rationale: sibling-slack models a chained *join* band across *this
  scope's own FROM join graph*; an expr-scope subquery (correlated or not) is never a member of
  that graph at this node, so folding it in is unsound conservatism, not the criterion 1
  equivalence the plan intends (that equivalence is with an *uncorrelated cross-joined derived
  table* — a correlated subquery's only valid literal rewrite is `LATERAL`, which the walk does not
  model, so equality there was never claimed). Spec text updated to match; `tracer_propagation`
  passes unmodified.
- 2026-09-05: empty-`ExprScope`-verdict default for the per-column fold is `Determinism::Clean` /
  `Comparability::Comparable`, not the fail-closed pessimistic default — an `EXISTS (SELECT *
  FROM t)` scope contributes no per-column facts at all (its item is skipped as a wildcard), and
  treating "no columns to compare" as taint would be an ungrounded pessimism, not a proof.

**For the next planner:**
- Phase 3 (skew, footprint-trajectory) is now more clearly motivated than "still bounded": fixing
  `own_region_text` to exclude every `SUBQUERY` subtree (this phase, for `ReachTransfer`
  correctness) means `SkewTransfer` is now **blind** to Form B content living only inside an
  expression-position subquery, not merely double-counting it as before phase 1. Doc comment on
  `SkewTransfer` records this; phase 3 should treat it as a real (if temporary) regression to
  close, not just an outstanding TODO.
- Phase 3 should re-examine whether `TrajectoryTransfer`/`SkewTransfer` want the same
  read-vs-join-sibling split this phase introduced for `ReachTransfer`, since the same
  chained-band-vs-subquery distinction likely applies to skew's Form B derivation too.
- Nothing else left the outcome; the `bronze_events`/`conversions` cross-contamination fixture is
  now implicitly covered by `tracer_propagation.rs` (unmodified, still asserting *tight* bounds) —
  a useful regression fence for phase 3's own sibling-slack decisions.

**Gates:**
- `cargo test -p smelt-logical --quiet` — 743+ new/existing tests green (incl. 10 new).
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 4 passed.
- `cargo test -p smelt-planner --quiet` — all green.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `cargo test -p smelt-runtime --test tracer_propagation --quiet` — 6 passed (verified unaffected
  after the sibling-slack deviation above; failed with literal sibling-slack inclusion).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
