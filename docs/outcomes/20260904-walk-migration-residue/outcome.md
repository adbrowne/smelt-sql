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
- The membership-sensitivity **closure-pruning** pass (`maintenance/grouping.rs`'s
  `closure_pruned_source`) consulting the declared-RI route. It is a column-provenance pruning
  pass, not a maintenance-cell admission route, and `model_properties.md` §Semantics declares its
  declared-RI exclusion deliberate, conditioning any widening on a paired probe dispatch that does
  not exist for a prune — a narrowing decision, not migration backlog.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Expression-position subqueries and parenthesised join groups normalized as walk nodes; children convention extended with a behaviour-preserving expression-scope tail; every `Transfer` impl audited | done |
| 2 | Bound/reach and grain transfers consume expression-scope verdicts; inline-equivalence property test | done |
| 3 | Skew and trajectory transfers consume the expression-scope tail (phase 1's three explicit bounds retired) | done |
| 4 | Cumulative classifier: both whole-SQL scans (`OVER(` and the nondeterministic-function loop) onto the walk as leaf classifiers; `walk_coverage` covers the file and catches the case-folded-variable scan form | done |
| 5 | Declared-RI closure **and** declared unique-key facts reach every `JoinContext`-taking maintenance-cell route (model-edge cells, per-group repair admission); per-route fixtures + a structural empty-context check | done |
| 6 | Retire the whole-SQL flat-scan bound floor in `derive_model_bounds` (its recorded justification — expression subqueries are not walk nodes — died in phase 1); correct the stale `has_unsupported` fallback comment | done |
| 7 | Delete the divergence bullets this outcome closed; `/smelt:validate model_properties`; all gates green | pending |

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

- 2026-09-05 (implement, phase 4): shipped — both `classify_cumulative` whole-SQL scans replaced
  by `walk::first_scope_hit` over two new leaf classifiers (`scope_has_window_function`,
  `scope_nondeterministic_fn`), built on a new shared `visit_own_region` (factored out of
  `own_region_text`'s pruning traversal). `walk_coverage`'s `KNOWN_NONCOMPLIANT` is now empty and
  `is_raw_scan_line` catches the case-folded-variable scan form. Spec-first correction:
  `incremental_shapes.md` narrowed `KeyedForbidsWindowFunctions`/`KeyedForbidsNondeterministic` to
  "the outer SELECT", but the implementation always refused on any scope — the spec text is
  corrected to match rather than the refusal narrowed. Confirmed bare `CURRENT_TIMESTAMP` (no
  parens) parses as a column reference, not a call, so the new `FUNCTION_CALL`-based classifier
  is behaviour-preserving with no extra case needed (the old scan's pattern always required a
  trailing `(` too). See `phases/04-summary.md`.

- 2026-09-05 (plan, phase 5): read criterion 3's "route" as a maintenance-**cell admission** route
  and enumerated them: `append_model_edge_cells` (empty RI input *and* a context carrying no
  external source's declared `unique_key`), `repair::admit_per_group_recompute` (a literal
  `JoinContext::new()`), and the already-wired `mutation_enrichment_closure`. The second of those
  was not previously named anywhere in the outcome; it is folded into phase 5 rather than deferred,
  and the row text is widened to say so. `grouping.rs`'s closure-pruning pass is the one
  JoinContext-taking site deliberately left out, recorded under Out of scope with its rationale.
- 2026-09-05 (plan, phase 5): a model edge has no referential-integrity declaration of its own —
  models declare no `referential_integrity:`, and completeness is not derivable from an upstream's
  SQL — so wiring the source-keyed RI map into the model-edge route by edge name would be inert
  wiring with no constructible fixture. The reachable fact is instead the **external sources joined
  in the same scope**, which that route ignores today: the phase widens its P1 verdict to an AND
  over every enrichment relation in the scope, each judged with its own declared facts. Adding a
  model-level `referential_integrity:` surface was rejected as a product decision outside this
  outcome's migration-backlog mandate.

- 2026-09-05 (implement, phase 5): shipped — `append_model_edge_cells`'s shared P1 AND now
  folds every external source actually joined in the scope (not only model edges), each judged
  with its own declared `unique_key`/`referential_integrity` via a unioned `JoinContext`
  (`JoinContext::union`, new); `repair::admit_per_group_recompute` takes a real `join: &JoinContext`
  instead of a literal empty one, wired from its production caller's `inputs.sources`. New
  structural gate `join_context_reach.rs` requires every production `JoinContext::new()` in
  `src/maintenance`/`src/analysis` to carry an inline classification tag — zero unclassified
  survivors. Two pre-existing sites (`rules/cumulative.rs`'s once-write route,
  `locality.rs`'s route-2 FD check) were found to read `has_fan_out_join` off an always-empty
  context but have no declared facts to widen with today — recorded as follow-up, not fixed
  (neither is a model-edge/repair admission route in criterion 3's sense). See
  `phases/05-summary.md`.

- 2026-09-05 (plan, phase 6): reshape — inserted a phase ahead of the doc cleanup. Reading the
  MP-03 bullet's residue against the code turned up a live whole-SQL scan the outcome's own work
  invalidated: `derive_model_bounds`'s **flat-scan floor** merges `derive_bound_for_source(sql, ..)`
  — a scan over the entire model text — into every source's walk verdict, and commit `20e74879`
  ("reach keeps the flat-scan floor — expression-subquery reference sites stay covered",
  2026-07-08) records its sole justification as "a source referenced inside an expression-position
  subquery (which is not a walk node) would carry only its leaf-path reach". Phase 1 made those
  scopes walk nodes, so the justification is void, yet a composition-relevant verdict (a bound)
  is still floored by a whole-SQL scan. `walk_coverage` cannot see it — the gate matches
  `.contains("` string-literal scans, not a call taking the whole `sql`. Deleting the MP-03 bullet
  (criterion 4) while that floor stands would assert something false, so the work gets a row
  rather than a "for the next planner" note. Nothing left the outcome; the doc-cleanup row
  renumbered 6 → 7.
- 2026-09-05 (plan, phase 6): scoped as *retire or narrow*, not "delete and hope". The floor's
  `merge` takes max, so removing it can only narrow bounds — every narrowing the corpus exposes is
  either a real walk gap (fixed in the walk) or one named, verified shape that keeps a narrow,
  classified floor. Judged the temporal-proof clause of the MP-03 bullet to be MP-02, already
  under Out of scope, and left it there.
- 2026-09-05 (plan, phase 6): note for phase 7 — MP-13's bullet must **not** simply be deleted.
  Its original claim (the probe does not consult declared lateness) was retired by the
  2026-09-04 lateness decision; the bullet standing in §Known Divergences today is a *different*,
  live gap (late append vs violation) already scheduled onto
  `docs/outcomes/20260904-decision-residue/outcome.md`. Phase 7 verifies each bullet against the
  code and deletes only what is false.

- 2026-09-05 (implement, phase 6): shipped — the per-source flat-scan floor is gone;
  walk-only equals floored across the full existing test corpus (verified before deletion). Two
  real bugs surfaced and were fixed rather than papered over with a wider floor: (1)
  `ReachTransfer::leaf` never resolved the maintenance-plan subsystem's "sources."-stripped
  bare-name `BoundContext` convention, silently masked by the floor's table-identity blindness for
  as long as the floor existed; (2) a source read only as a table-valued function call's argument
  (`smelt.functions.sessionize(source => smelt.silver.x, …)`, `examples/web_analytics`'s real
  `silver.sessions` shape) is structurally invisible to the walk under any tree shape, not just an
  `Unsupported` one — kept as one named, tested leaf classifier per the plan's task-5 escape hatch,
  plus a `model_properties.md` §Known Divergences line. 22 pre-existing unit tests and 2 integration
  tests had fixture SQL that only ever passed because the floor was blind to table names; fixed to
  reference sources the way every real caller does. See `phases/06-summary.md`.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
