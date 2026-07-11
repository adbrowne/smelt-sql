# Generative maintenance conformance: property-testing the equivalence invariant over generated models

- **Date**: 2026-07-11
- **Status**: research / design (predecessor to a phased implementation plan)
- **Author**: Andrew (with Claude)
- **Motivates**: a standing, generative conformance gate for the maintenance-plan family
  (`docs/specs/maintenance_plan.md` §"The equivalence invariant") — the safety net the recently
  merged maintenance work currently lacks at the *generated-model* level.
- **Successor to**: `docs/research/20260705-property-discovery-loop.md` (the Link-0/A/B/C
  research loop). That loop built the execution harness and proved the method on a hand-written
  model catalogue; this design generalizes the *model* axis to property-based generation and
  turns the loop's disposable probes into a permanent gate.
- **Related specs**: `maintenance_plan.md`, `model_properties.md`, `model_transforms.md`,
  `models.md`, `sources.md`, `batched_models.md`, `keyed_models.md`.

---

## 1. Problem

The maintenance plan makes one safety promise — the equivalence invariant: for every
maintained (non-`full`) model, `incremental_state(S) == full_refresh(inputs ∈ S)` for the
processed-input set `S`, under *any* valid sequence of runs and input arrivals. Everything else
(admission, clamps, ledger grading, technique choice, propagation) exists in service of that
invariant.

Today the invariant is executably tested only over a **hand-enumerated model catalogue**
(`crates/smelt-maintenance-testkit/src/model_shapes.rs`, ~16 shapes) driven by
proptest-generated run schedules. The models themselves — the axis with the combinatorial
surface (construct × source posture × grain × frontmatter) — are never generated. The
coverage-matrix inventory gate
(`crates/smelt-logical/tests/maintenance_plan_conformance.rs::coverage_matrix_is_inhabited`)
is honest about this: of ~100 inhabited `(construct × source-property)` cells, ~13 are
`CLAIMED` by an executable test; the rest are named `KNOWN_GAPS`.

The goal: **generate models, ask the real derivation what properties hold and what plan it
admits, simulate input data arriving over multiple runs (including simulated change feeds),
and verify every admitted plan upholds the invariant against a full refresh — on real DuckDB,
as a standing gate.**

## 2. What exists, and the precise gaps

The property-discovery loop (2026-07-05) already built the hard parts, all reusable:

| Piece | Where | Status |
|---|---|---|
| Real-pipeline driver (in-process `execute_project` over temp DuckDB, no hand-injected `WHERE`, re-discovers models per run) | `smelt-maintenance-testkit/src/link_c_harness.rs` (`LinkCProject`, `SqlCapturingReporter`) | reuse as-is |
| Run-schedule generator with between-run source mutation + per-step snapshots | `smelt-maintenance-testkit/src/run_schedule.rs` | extend (schema-generic, new step kinds) |
| Multiset equivalence oracle (`EXCEPT ALL` both directions) | `smelt-maintenance-testkit/src/oracle.rs` | reuse as-is |
| Model-shape catalogue (~16 hand-written shapes) | `smelt-maintenance-testkit/src/model_shapes.rs` | absorb into pinned recipes (§11) |
| Standing execution gate (runs on every `cargo test`) | `crates/smelt-cli/tests/property_discovery/` | superseded gradually (§11) |
| Abstract fold-vs-batch pre-filter (Link A), clamp-probe diagnostics (Link B) | `crates/smelt-db/tests/proptests/maintenance_link_{a,b}*.rs` | keep as diagnostic pre-filters |
| Typed expression/column generators, DuckDB oracle plumbing, data generators | `crates/smelt-db/tests/prop_helpers/` | source of parts (value literals, oracle plumbing) |

The gaps this design fills:

1. **No `Strategy<ModelShape>`** — model SQL is hand-authored per catalogue cell; nothing
   probes the space between cells.
2. **No schema-generic schedule/data generation** — `run_schedule.rs` is hard-coded to
   `events(d DATE, id BIGINT, val DOUBLE)`.
3. **No generic full-refresh baseline** — every probe hand-writes its own oracle query
   (e.g. `g_01`'s `full_refresh_total()`); nothing derives one from an arbitrary model.
4. **No change-feed generator** — named a Phase-0 sub-task in the 2026-07-05 design, never
   built; `change_feed` sources today collapse to `MutableSnapshot`
   (`crates/smelt-db/src/queries/maintenance.rs::source_facts`) and admit recompute only.
5. **No plan-claim checking** — the derived plan asserts clamps, write windows, column-group
   sensitivity, ledger grading; no test checks those claims against observed runtime behaviour
   on models it didn't hand-pick.
6. **Zero integration coverage of reconciliation-ledger persistence** — `execute_project`
   writes `.smelt/reconciliation.json` on every region recompute
   (`crates/smelt-runtime/src/execute.rs` recompute-reset call site), but no test runs twice
   and inspects the persisted store.
7. **~20 executing probes are `EXPERIMENTAL(property-discovery): disposable`** with no gate
   against silent deletion; only `g_10` and `coverage_matrix_gaps.rs` graduated.

## 3. Design overview

One new axis on the existing harness, not a new harness. Per generated case:

```
Strategy<ModelRecipe> ──render──► staged project (smelt.yml + models/*.sql + models/sources/*.yml)
        │
        ▼
 real derivation (smelt_db::queries::maintenance::derive_model_maintenance_plan)
        │
        ├─ REFUSED  → assert a named Maintenance* diagnostic is present (fail-loud check);
        │             record cell in the over-refusal ledger; case passes
        │
        └─ ADMITTED → Strategy<RunSchedule> (schema-generic, recipe- and plan-aware)
                        → RunScheduleDriver (real execute_project per step; per-step snapshots)
                        → equivalence oracle (§6) + plan-claim probes (§7)
```

Both branches are load-bearing. The admitted branch hunts **unsound acceptances** (the safety
headline: smelt maintains something a run sequence breaks). The refused branch enforces the
**fail-loud discipline** (an unrecognised construct must refuse with a named diagnostic, never
silently degrade) and feeds an **over-refusal ledger** (completeness regressions, lower
severity, tracked not gated).

## 4. `ModelRecipe` — generating models as typed data

A recipe is a small typed value, not SQL text. Fields:

- **Sources** (1–3): each with a posture (`append_only` | `mutable_snapshot` | `change_feed`),
  clocked or unclocked (`timeseries:` block present or absent), unique key (none | single |
  composite), declared `mutation_profile` sub-facts, optional declared lateness, and a typed
  column schema (drawn from a small pool: the clock column, key columns, integer payload
  columns, a low-cardinality dimension column).
- **Body construct**, drawn from the coverage-matrix construct axis: pass-through · filter ·
  additive aggregate (`SUM`/`COUNT`) · idempotent aggregate (`MIN`/`MAX`/`BOOL_OR`) ·
  decomposed aggregate (`AVG`) · holistic aggregate (`MEDIAN`/`COUNT(DISTINCT)`) · inner-join
  enrichment · left join · window frame (`RANGE`/`ROWS`, bounded/unbounded) · `UNION ALL` ·
  CTE nesting/rename · correlated `EXISTS` · self-reference (running balance). Compositions
  (e.g. aggregate-over-join, frame-inside-CTE) are generated with bounded depth.
- **Output shape**: `grain: partition` or `grain: key`, with matching frontmatter
  (`timeseries:`, `batched.unique_key`, keyed key columns).
- **Frontmatter knobs**: optionally a `maintenance.cells[].technique` pin (for the
  interchangeability probe, §7), `scan_bounds` acceptances (`allow_full_scan` where the recipe
  knows the construct needs it), `horizon_ceiling`.
- **Adversarial leaf pool** (refusal-branch fuel): opaque/unrecognised function calls,
  `INTERSECT`/`EXCEPT`, row-nondeterministic functions in payload vs skeleton positions,
  symbolic intervals (`'1 month'`) in bound-relevant positions. These *should* refuse or
  degrade to whole-model recompute; the case asserts the named diagnostic / conservative plan.

Properties of the representation:

- **Valid-by-construction**: typed columns and resolvable refs, so generated SQL parses and
  type-checks; a recipe that trips non-maintenance diagnostics is a generator bug, surfaced by
  a self-check, never silently discarded.
- **Structural shrinking**: proptest shrinks the recipe (drop a join, simplify a combiner,
  shorten the schedule) rather than mangling SQL text. A shrunk failure is pinned as an
  explicit regression test — the repo's standing rule for property-test failures.
- **Matrix-aware**: each recipe knows which coverage-matrix cell(s) it inhabits
  (`construct × source-property`), so generated coverage is measured against
  `coverage_matrix_is_inhabited`'s inventory instead of floating free (§10).
- **Renders once, serves three**: the same rendering produces the model file, the staged
  source YAMLs, and the oracle query (§6).

## 5. Data and schedule generation

**Row data** becomes schema-generic (recipe-driven), replacing the fixed `events(d,id,val)`:

- **Numeric payload discipline**: integer-valued payloads with bounded magnitude only. Additive
  folds over doubles are order-sensitive in general; integer-valued sums are bit-exact well
  under 2^53, so incremental-vs-full comparisons via `EXCEPT ALL` stay exact regardless of fold
  order. Variance/stddev-class combiners are excluded from the v1 pool for the same reason
  (adding them later requires an explicit tolerance-scoped comparison mode, which is its own
  design decision, not a default).
- **Boundary-value placement**: the generator reads the *derived plan's* scan clamp and
  deliberately lands rows just-inside / at / just-outside the claimed reach. An under-derived
  clamp only diverges observably when data sits at the boundary; uniform random dates rarely
  put it there.
- **Key-recurrence control**: keyed recipes generate schedules with deliberate key re-touch
  across windows (the interesting case for merges) and, where ordering-sensitive combiners
  (`MAX_BY`-family) are generated, ordering keys are made unique by construction so the
  documented ties carve-out cannot fire spuriously (§6).

**Schedule steps** extend the existing four (`AdvanceWindowAndRun`, `AppendLateRow`,
`InPlaceUpdate`, `InPlaceDelete`) with:

- **Re-run of an already-processed window** (re-delivery / idempotency — the never-fold-twice
  obligation's trigger);
- **Explicit backfill** of a named region;
- **`full_refresh: true` interleave** (must reset coverage + ledger state correctly);
- **Multi-source interleave** (fact append and dimension mutation inside one schedule — keys
  the per-input scope-map dispatch);
- **Window-order permutation** (order/set-determinacy corollary);
- **Catch-up step**: every generated schedule ends by re-running every window affected by
  late/mutating steps, so a settled point exists for the settled-point oracle mode (§6).

The `MutationProfile` self-check (`check_profile`) extends to every new step kind: a schedule's
*declared* posture is verified against its actual steps, never trusted — an unverified label
silently poisons a verdict (finding F7 of the 2026-07-05 review).

**Simulated change feed.** Nothing in production consumes a feed today; deltas are discovered
by interval-diffing a clocked source's landings (`smelt_state::landed_deltas`), and a declared
`change_feed` collapses to `MutableSnapshot` for admission. The harness nonetheless adds a
`SimulatedChangeFeed` step family now: every mutation step against a feed-declared source is
applied to the base table *and* appended as an `(op, key, payload, seq)` row to a staged feed
table. v1 asserts today's contract — only full-input re-derivation is admitted for such
sources; equivalence holds via the recompute path — and the feed bookkeeping means the day
feed-consuming folds land (offset-based delta detection, retraction-carrying deltas into
invertible combiners), generative coverage for them already exists rather than starting from
zero. Tombstone/retraction step kinds are part of this family, gated to feed-declared sources.

## 6. The equivalence oracle, generalized

The invariant's two forms (spec §"The equivalence invariant", research-doc N3) become two
oracle modes selected per source posture, both fed by an **S-tracker** built on the existing
per-step snapshots:

- **Oracle query**: rendered from the recipe — the same SQL body with `smelt.sources.*`
  references swapped for physical table names — evaluated directly on a `duckdb::Connection`.
  This is deliberately independent of smelt's compile/execute pipeline: the body is shared *by
  construction* (that is the invariant's own statement — same SQL, full inputs), and everything
  downstream of the body — bound derivation, filter injection, batching, technique dispatch,
  emitters, state — is under differential test.
- **Append-only sources → S-restricted oracle, asserted after every run step.** The S-tracker
  records `(window, source-snapshot)` per run; `S_k` = the rows visible-in-window at some
  processed run ≤ k. The oracle materializes `S_k` into temp tables and evaluates the body over
  them. Asserting every step catches both failure directions: a row in `S` missing from the
  maintained state (under-scan, clamp too narrow) *and* a row outside `S` present in it
  (over-scan — the maintained state reflecting inputs it never legitimately processed). A
  genuinely late row is outside `S` until its window is re-run — exactly the spec's
  "silently excluded past the horizon" semantics, so lateness needs no special-casing here.
- **Mutable sources → settled-point oracle.** An in-place update cannot be un-seen by a filter,
  so no monotone `S` exists; "full refresh over source state at step k" is the well-posed form,
  and full equality is asserted at **settled points** — after catch-up steps, when no mutation
  is outstanding against an unprocessed region. Between settled points the harness asserts the
  weaker expected-staleness contract non-fatally (the g_04/sc_2 pattern, generalized).
- **Mixed models** (append-only driving fact + mutable dimension): the S-restriction applies to
  the driving source; the dimension contributes its current state. An outstanding dimension
  mutation flips the case to settled-point mode until a catch-up run covers the affected
  regions. The S-tracker owns this bookkeeping; postures come from the recipe and are
  self-checked (§5).
- **Keyed-grain carve-outs** (the two named ones, spec §"Two named carve-outs"): ordering-key
  ties are sidestepped by generator discipline (unique ordering keys by construction), so exact
  equivalence is assertable; retained-departed-keys is implemented as the documented oracle
  adjustment (oracle rows ∪ retained departed keys) when snapshot-reconcile schedules generate
  deletes.
- **Diff scope**: all output columns, always — recipes place nondeterministic functions only in
  the adversarial refusal pool, never in admitted-branch payloads, so no column exclusion
  machinery is needed.

## 7. Plan-claim probes — checking that derived properties hold

The derived plan is pure data making runtime-checkable claims. End-state equivalence alone can
miss a claim being wrong in a compensating way; each claim gets a direct probe:

| Plan claim | Probe |
|---|---|
| Scan clamp `(clock_col, before, after)` per source | The compiled SQL captured by `SqlCapturingReporter` carries exactly the claimed filter (plan-vs-execution consistency); boundary-placed data (§5) makes an under-derived clamp diverge in the oracle |
| Write window = output window | Output rows outside the write window are byte-unchanged across a run (snapshot-and-compare of the complement region) |
| Column-group mutation-sensitivity | A schedule mutating only source X leaves columns in groups *not* sensitive to X unchanged (meaningful for column-scoped-merge cells; a region recompute legitimately rewrites whole rows) |
| Never-fold-twice / ledger grading | Re-delivering a window is refused (`KeyedReprocessedWindow`) or provably not double-counted, per grading (`Grade::Additive` vs `Idempotent`); **and** the persisted `.smelt/reconciliation.json` is inspected across runs — fold extends entries, recompute-reset replaces intersecting entries with exactly the input read — closing gap 6 of §2 |
| Technique interchangeability | For a cell admitting both families, the same seed + schedule runs once with `maintenance.cells[].technique: fold` and once with `recompute`; final states must be identical (bit-preserving for faithful-idempotent columns; state-preserving modulo ledger for additive — fold-then-recompute is the safe order per the spec) |
| Order/set-determinacy | Valid window-order permutations of one schedule converge to identical final states |

Probes are per-case opt-in by what the plan admits — a probe that doesn't apply to a cell is
skipped explicitly (counted, so a probe that never fires is a visible generator gap, not a
silent one).

## 8. Refusals, fail-loudness, and generator health

- **Refused recipe ⇒ named diagnostic.** The refusal branch asserts one of the `Maintenance*`
  family (or the relevant admission diagnostic) is actually emitted — never a silent fallback.
  A refusal with no diagnostic is a fail-loud-discipline bug and fails the case.
- **Over-refusal ledger.** Refused cells are recorded per matrix cell. This is *tracked, not
  gated* (over-conservatism is safe); the ledger gives the completeness story a measurable
  surface, and a cell the spec claims admissible showing 100% refusal is a finding.
- **Generator health (reachability smoke test).** A deterministic-seeded sample asserts:
  every technique family (`DeleteInsert`, keyed fold, `ColumnScopedMerge`), trigger class, and
  grain is actually reached; the admission rate stays above a floor. A generator drifting to
  95%-refused recipes silently tests nothing — this is the anti-vacuity rule, made structural
  (pattern copied from `type_property_tests.rs::reachability`).

## 9. Multi-model DAG phase

Generated 2–3 node graphs (chain, diamond, fan-out) wire single-model recipes via `smelt.ref`:

- **Forward-propagation sufficiency**: for generated landed deltas, run exactly the per-edge
  dirty regions `plan_since_upstream` computes and compare *every* node to full refresh.
  (`since_upstream_propagation.rs` today stops at edge derivation; `since_upstream.rs` covers
  fixed fixtures — neither executes-and-compares over generated graphs.)
- **Backward resolution**: stage exactly the `--include-upstreams` resolved slices, build
  bottom-up, assert the target period equals a build over complete history.
- **The payload-leak family**: an upstream payload column consumed in a downstream skeleton
  position (`JOIN … ON` / `GROUP BY`) — the fixture family single-model tests structurally
  cannot reach (2026-07-05 §4).
- Adjointness (`forward(backward(P)) ⊇ P`) stays owned by the pure
  `maintenance_propagation_adjoint.rs` suite; the DAG phase tests execution, not the math.

## 10. CI, governance, soak

- **Standing gate**: a new `cargo test -p smelt-cli --test maintenance_conformance` target,
  deterministic-seeded, small-N (~10–20 generated cases per construct pool; each case is a
  staged project + several `execute_project` runs, ~1–3 s), on every `cargo test` — same
  posture as today's `property_discovery` gate. An env knob (`SMELT_CONFORMANCE_CASES`, mirroring
  `PROPTEST_CASES`) scales depth locally.
- **Soak**: a nightly/label CI job at high case counts, plus an autonomy-loop sub-plan for long
  soak sessions. Every shrunk failure graduates to a pinned recipe + schedule regression test.
- **Divergence registry**: named, statused entries (`ByDesign` / `KnownBug` / `BackendSpecific`)
  for accepted divergences, keyed by recipe fingerprint + probe, with a staleness report for
  entries that never fire — the type-oracle governance pattern
  (`prop_helpers/divergences.rs`), applied to maintenance.
- **Matrix convergence**: the coverage matrix gains a per-cell "generatively covered"
  annotation maintained by the reachability sample; `CLAIMED` entries may cite pinned recipes.
  The matrix stays the single legible inventory; the generator is measured against it.

## 11. Disposition of the existing test surface

| Suite | Fate | Why |
|---|---|---|
| `statement_parity.rs`, `maintenance_plan_conformance.rs` (+ coverage matrix), tracer/evolution/propagation suites, `maintenance_propagation_adjoint.rs`, `since_upstream.rs` / `include_upstreams.rs`, `smelt-state` ledger unit tests, `fold_ledger_delta` backend tests | **Keep** | Different layers: byte-level executed-vs-emitted parity, pure derivation regression floor, graph math, CLI wiring. The generative harness sits above them, not instead of them |
| `smelt-maintenance-testkit` (harness, oracle, schedule) | **Keep + extend** | The spine of this design |
| `model_shapes.rs` catalogue | **Absorb**: each shape becomes a named *pinned recipe* rendered by the recipe path; the file retires once a parity check proves pinned-recipe rendering reproduces each catalogue shape's coverage | One rendering path; catalogue legibility survives as the pinned-recipe corpus |
| ~20 disposable `g_*`/`sc_*`/`p0_*` probes | **Graduate then retire**: each probe's seeded hazard schedule becomes a pinned recipe + schedule in the corpus; the probe files are then deleted | They were explicitly disposable research probes; their *hazards* are the durable value |
| `crates/smelt-cli/tests/incremental/` (hand-injected `WHERE`) | **Reframe, keep**: documented as backend-strategy-layer tests (`Backend::execute_model_incremental` given a filter); overlap with the new harness trimmed | Real coverage at a different layer; wrong tool for planner-derivation claims (the F5 finding), which the doc header will state |
| `maintenance_link_a.rs` / `maintenance_link_b*.rs` | **Keep** as diagnostic pre-filters; a later phase may point link-b's clamp-probe at generated recipes | Fast refutation + failure localization; never a green gate |

## 12. Phasing outline (for the implementation plan)

1. **Recipe substrate**: `ModelRecipe` + rendering + admitted/refused protocol + S-tracked
   oracle; partition-grain append-only pool (pass-through, filter, aggregates); standing gate +
   reachability smoke test + divergence registry skeleton.
2. **Mutable sources**: settled-point oracle mode, column-scoped merge pool, sensitivity probe,
   multi-source interleave.
3. **Keyed grain**: fold MERGE pool, ledger probes incl. persisted `.smelt/` inspection,
   carve-out-aware oracle, interchangeability pins.
4. **Schedule enrichment**: re-delivery, backfill, `full_refresh` interleave, window-order
   permutation, boundary-value placement.
5. **Simulated change feed**: feed-table step family, refusal conformance, over-refusal ledger.
6. **Definition-change steps**: model file rewritten between runs (column add → backfill /
   evolution triggers; interval-store hash invalidation semantics).
7. **Generated DAGs**: propagation sufficiency + backward resolution equivalence +
   payload-leak family.
8. **Graduation & consolidation**: catalogue → pinned recipes, probe retirement, spec updates
   (`maintenance_plan.md` §References → Tests; `architecture.md` standing-gate entry), soak
   wiring (nightly job + autonomy-loop sub-plan).

Each phase lands with its own standing-gate extension and is independently valuable; the plan
should be executable by the autonomy loop.

## 13. Rejected alternatives

- **Abstract-simulator-first** (extend Link A into a full plan simulator, bind to reality via
  statement parity): thousands of cases/sec, but the headline bug class — wrong derived clamp,
  wrong runtime dispatch, wrong persisted state — lives precisely in the gap between the
  abstraction and the real pipeline. The 2026-07-05 review (F1) already killed abstract-only
  designs for this reason; Link A stays a pre-filter.
- **Enumerate harder** (lift all matrix `KNOWN_GAPS` to `CLAIMED` by hand): bounded and
  legible but tests only cells someone thought of; kept as the coverage ledger the generator
  is measured against, not the method.
- **Raw-SQL fuzzing** (generate/mutate SQL text): poor shrinking, mostly-invalid corpus,
  no recipe-level knowledge for oracle rendering or matrix mapping. Typed recipes give
  valid-by-construction models and structural minimization.
- **Full-refresh oracle via a second `execute_project` run**: shares the whole compile
  pipeline with the system under test, weakening independence; direct evaluation of the
  rendered body over snapshot tables tests everything downstream of the shared SQL body, which
  is the honest boundary (the invariant itself is stated over the shared body).

## 14. Open questions

- **Tolerance-mode combiners** (variance/stddev, true-float payloads): excluded from v1;
  admitting them needs an explicit approximate-comparison mode with its own soundness argument.
- **Hour granularity / sub-day axes**: propagation is day-ordinal today; recipes stay
  day-grained until the axis generalizes.
- **`materialized_view` shapes**: the invariant is discharged by the engine's native IVM, not
  the smelt oracle — out of scope for this harness beyond asserting the refusal/hard-error
  surface on backends without support.
- **Spark**: the harness is DuckDB-first (the testkit's backend factory is DuckDB). The recipe
  layer is backend-agnostic by construction; a Spark `BackendFactory` variant is future work
  gated on the Spark-parity job's infrastructure.
