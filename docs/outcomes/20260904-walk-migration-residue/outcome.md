# Outcome: Walk-migration residue — the composition walk is the sole source of every property

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md` rows MP-03, MP-05, MP-11, MP-13 (each classified "migration backlog, not a design question"); `docs/specs/model_properties.md` §Known Divergences
**Spec anchors:** `docs/specs/model_properties.md` §Constraints "Composition happens in the walk, not in scans", `docs/specs/architecture.md` §"Property composition walk rule"

## The outcome

Every composition-relevant model-property verdict comes from the shared bottom-up walk in
`smelt-logical`'s `analysis/walk.rs`. Scopes inside expression-position subqueries are walk
nodes. The cumulative classifier's whole-SQL `OVER(` check is either a walk-invoked leaf
classifier or gone. Every maintenance-cell route that can consult a declared referential-integrity
closure does so, not only the source-enrichment route. The `walk_coverage` gate, not a doc comment, is what
says the rule holds.

## Success criteria (checkable)

1. Expression-position (scalar and `EXISTS`) subqueries and redundantly-parenthesised derived
   tables are enumerated as walk nodes; a property test shows a bound/reach/grain verdict for a
   model reading such a scope equals the verdict for the same model with the scope inlined.
2. `classify_cumulative`'s `OVER(`/`OVER (` text check is classified onto the walk as a leaf
   classifier over one bounded node's text, or deleted; no whole-SQL scan remains in
   `rules/cumulative.rs` (grep-asserted by `walk_coverage`).
3. Every maintenance-cell admission route that takes a `JoinContext` receives the declared-RI
   closure map (none passes an empty map); a fixture that admits only with the closure present
   exists per route.
4. `model_properties.md` §Known Divergences bullets for MP-03, MP-05, MP-11 and MP-13 are
   deleted; `/smelt:validate model_properties` clean.
5. `cargo test -p smelt-logical --test walk_coverage`, `maintenance_conformance`,
   `statement_parity` and `verify-phase.sh` green; no new whole-text scan introduced.

## Out of scope

- Merging the `EffectiveWindow` and `BoundResult` walks (MP-02, an architecture decision).
- Anything to do with declared lateness. Decided 2026-09-04 that lateness is orchestration-only
  and never a plan or probe input (`docs/research/20260904-decision-track.md`); the former
  criterion 4 (probe consults lateness) was removed for that reason, and the probe's
  late-append classification is `docs/outcomes/20260904-decision-residue/outcome.md`'s.
- Widening skeleton-source closure beyond non-aggregating scopes (MP-10, admission width).
- `SourceUniqueKeyViolated`'s missing emitter (MP-14, undecided).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Expression-position subqueries and parenthesised join groups normalized as walk nodes; children convention extended with a behaviour-preserving expression-scope tail; every `Transfer` impl audited | done |
| 2 | Bound/reach and grain transfers consume expression-scope verdicts; inline-equivalence property test | done |
| 3 | Skew and trajectory transfers consume the expression-scope tail (phase 1's three explicit bounds retired) | done |
| 4 | Cumulative classifier: both whole-SQL scans (`OVER(` and the nondeterministic-function loop) onto the walk as leaf classifiers; `walk_coverage` covers the file and catches the case-folded-variable scan form | planned |
| 5 | Declared-RI closure reaches every `JoinContext`-taking maintenance-cell route; per-route fixtures | pending |
| 6 | Delete the four divergence bullets; `/smelt:validate model_properties`; all gates green | pending |

## Decision log

- 2026-09-05 (plan, phase 1): probed the normalizer directly — `FROM ((SELECT …)) AS t` **already**
  nests as a `Derived` node (the parser's `Subquery::select_stmt` unwraps redundant parens since
  the divergence bullet was written), so criterion 1's "parenthesised derived table" half is
  already met. The live `Unsupported` in that family is a parenthesised **join group**
  (`FROM (a JOIN b ON …)`), which is what phase 1 now covers; the stale
  `FROM ((SELECT …))` clause in `model_properties.md` §Known Divergences and in
  `walk.rs`'s `has_unsupported` doc comment is corrected as part of it.
- 2026-09-05 (plan, phase 1): split the original phase 1 in two. Expression-position subqueries
  are invisible to the walk today (`SELECT (SELECT max(b) FROM u) FROM t` yields exactly one leaf,
  `t`), and wiring them in touches all 11 `Transfer` impls' children-slice convention. Phase 1 is
  the behaviour-preserving structural half (normalize + fold + per-transfer audit); phase 2 is the
  verdict-affecting half (bound/reach/grain consume the new verdicts) plus criterion 1's
  inline-equivalence property test. Nothing left the outcome; later phases renumbered 3–5.

- 2026-09-05 (implement, phase 1): shipped — expression-position subqueries
  (scalar/`EXISTS`/`IN`/quantified) and parenthesised join groups are now walk nodes, folded as a
  documented `ctes ++ inputs ++ expr_scopes` children tail. Audited all 11 production `Transfer`
  impls; 3 needed an explicit bound to stay tail-safe (`TrajectoryTransfer`, `ReachTransfer`,
  `SkewTransfer`). Behaviour-preserving by design — no verdict changed, only reachability. See
  `phases/01-summary.md` for the full account.

- 2026-09-05 (plan, phase 2): added a phase for the **skew** and **trajectory** transfers. Phase 1
  left three transfers explicitly bounded to `ctes ++ inputs` (`ReachTransfer`, `SkewTransfer`,
  `TrajectoryTransfer`); phase 2 retires the reach bound (criterion 1 names bound/reach/grain),
  but skew and footprint-trajectory are composition-relevant verdicts too — the outcome's headline
  ("the walk is the sole source of every property") is unmet while their bound stands on a
  "not yet" comment. Rather than leave them as a silent deferral they get their own row (new
  phase 3); later phases renumbered 4-6. Nothing left the outcome.

- 2026-09-05 (implement, phase 2): shipped — `ReachTransfer` and `PropertyTransfer` consume the
  `expr_scopes` tail; `own_region_text` now excludes every `SUBQUERY` subtree (not just
  FROM-position ones) to stop double-counting once expr scopes became real walk nodes. Deviated
  from the plan's "participates in the sibling-slack computation identically" clause: expr-scope
  children merge into the per-source read map but are excluded from the join-sibling slack
  loop, which models chained bands across *this scope's own FROM join graph* — a real fixture
  (`tracer_propagation.rs`) showed literal inclusion spreading an unrelated subquery's reach onto
  an unconnected FROM input. Spec text updated to match the narrower, verified rule. See
  `phases/02-summary.md`.

- 2026-09-05 (plan, phase 3): no reshape — phase 2's summary confirmed the remaining work is
  exactly the two bounded transfers, and its `own_region_text` fix turned skew's gap from
  double-counting into blindness, so phase 3 closes a live regression rather than a TODO. Judged
  the phase-2 read-vs-join-sibling split inapplicable here: neither `SkewTransfer` nor
  `TrajectoryTransfer` has a sibling-slack computation, and both folds (`Skew::union`, parallel
  OR) widen conservatively, so the plan folds the whole children slice and keeps
  `tracer_propagation`/`footprint_reflection`/`since_upstream` as regression fences.

- 2026-09-05 (implement, phase 3): shipped — `SkewTransfer` folds the whole children slice
  unconditionally (no join-sibling carve-out, purely widening); `TrajectoryTransfer` folds an
  `expr_scopes` child only when its value flows into a select-list output column. Deviated from
  the plan's literal "unconditional `.any()` over the whole slice" for trajectory: that broke
  `window_inside_a_where_subquery_is_not_a_trajectory_of_the_outer_select`
  (`footprint_reflection.rs`) — a running fold inside a `WHERE`-clause scalar subquery is never a
  stored column, so per task 6 the widening was judged unsound at that node and narrowed, with the
  narrowing written into the spec delta. See `phases/03-summary.md`.

- 2026-09-05 (plan, phase 4): no phase added or removed. Widened phase 4's row to name the
  **second** whole-SQL scan in `classify_cumulative` — the `NONDETERMINISTIC_FUNCTIONS`
  `upper_sql.contains(&pattern)` loop. Criterion 2's headline names only the `OVER(` check, but
  its trailing clause ("no whole-SQL scan remains in `rules/cumulative.rs`") covers this one too,
  and it is the reason the file cannot simply leave `walk_coverage`'s skip-list: the gate's
  `is_raw_scan_line` only matches `.contains("` with a string literal, so removing the skip-list
  entry while that loop stands would make the gate pass over a live violation. Both migrations
  share one mechanism (a walk-composed scope-presence fold + a per-scope leaf classifier over the
  node's own region), so they belong in one phase.
- 2026-09-05 (plan, phase 4): the gate widening is cheap and precise, not a sweep. Probed the
  whole `smelt-logical` admission/proof surface for `<ident>.contains(` where `<ident>` is bound to
  a `.to_uppercase()`/`.to_lowercase()` result: exactly one site crate-wide, the very line task 5
  deletes. Every other `.contains(&x)` in those directories is collection membership, which the
  widened rule deliberately does not touch.
- 2026-09-05 (plan, phase 4): spec-first correction found while reading the anchors —
  `incremental_shapes.md` describes `KeyedForbidsWindowFunctions` as firing on "the outer SELECT",
  but the implementation has always refused on an `OVER` anywhere in the model text. The wider rule
  is the sound one (a window over a delta-filtered CTE scope is not the window over history), so
  the spec row is corrected to match rather than the refusal narrowed.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
