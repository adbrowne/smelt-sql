# Plan: Quality Grind Tier 2 — generator coverage, module consolidation, bench regression

**Date**: 2026-07-18
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) §"Constraints & Invariants" items 12–14, §"Layered single-ownership"; [`docs/specs/types.md`](../specs/types.md)
**Spec diff**: none — extends existing oracles/gates to deferred surface; no behaviour contract changes. Sources: `docs/TODO.md` §"Deferred" (generator gaps), `docs/ROADMAP.md` §"Deferred-Work Backlog" (consolidation, CI/Performance, CLI ergonomics).
**Master**: [`docs/plans/20260718-quality-grind.md`](20260718-quality-grind.md)
**Tracking PR / branch**: `worktree-roadmap_todo`
**Docs**: code+docs (phases state their doc surface; most are test/internal-only)

---

## Execution prompt (for a fresh Claude session)

Same conventions as [`20260718-quality-grind-t1.md`](20260718-quality-grind-t1.md):
red-green `/smelt:implement` per phase, `bash .claude/scripts/verify-phase.sh` as the
gate, atomic commits, block-and-continue on design decisions.

**Generator-phase convention (Phases 1–5).** These extend
`crates/smelt-db/tests/prop_helpers/generators.rs` following the established pattern
(TODO.md's completed items document it: add strategy → run
`PROPTEST_CASES=500 cargo test -p smelt-db --test type_property_tests prop_type_inference`
→ triage failures per the CLAUDE.md decision tree: known divergence → `divergences.rs`;
compatible → `type_comparison.rs`; else fix inference). A failure revealing a real
inference bug is a deliverable: pin it as an explicit regression test *before* fixing
(CLAUDE.md red-green rule). Also run the nullability oracle
(`nullability_property_tests`) since the generators are shared. `DUCKDB_LIB_DIR` +
`LD_LIBRARY_PATH` must be exported — if DuckDB tests skip, the phase is NOT verified;
mark blocked rather than claiming green.

---

## Context

Two threads: (a) the property-test generators' §Deferred items from `docs/TODO.md` —
each was deferred for a known syntactic reason, not difficulty; (b) two structural
paydowns from the ROADMAP deferred backlog — finishing the `smelt-logical` extraction
(the layered single-ownership invariant's remaining duplication) and root-causing the
cold-Salsa benchmark regression that has had the `Benchmarks` CI gate red on `main`
since 2026-07-09.

## Scope

### In scope
- Generators: aggregate FILTER, ordered-set aggregates (MEDIAN/MODE/PERCENTILE_CONT/
  PERCENTILE_DISC) + WITHIN GROUP for STRING_AGG/LISTAGG, two-column aggregates
  (CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE), ARRAY types, ROW/STRUCT constructors.
- smelt-planner↔smelt-logical duplicated-module consolidation (two phases).
- Cold-Salsa 2000-model regression: profile + report (fix only if unambiguous).
- CLI ergonomics: `smelt build --verbose`, `smelt test` silent-skip diagnostic.

### Explicitly deferred
- SET-operation / QUALIFY / PIVOT / lambda generator shapes (bigger query-shape work).
- Any registry-migration of the 30 legacy-match functions (needs richer signature language).
- Acting on the benchmark findings if the fix is non-obvious (decision D-QG-5 in the master).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | test(props): generate aggregate FILTER clauses against the DuckDB oracle | 2026-07-19 |
| 2     | pending |        |      |
| 3     | pending |        |      |
| 4     | pending |        |      |
| 5     | pending |        |      |
| 6     | pending |        |      |
| 7     | pending |        |      |
| 8     | pending |        |      |
| 9     | pending |        |      |

---

### Phase 1: aggregate FILTER clause in generators

**Goal.** `agg(x) FILTER (WHERE cond)` (already parsed) is randomly generated and
type-checked against DuckDB.

**Pre-conditions.** None.

**TDD tests to write first.**
- Reachability smoke test (the established pattern) proving FILTER-bearing aggregates
  are generated.
- If inference mishandles FILTER (e.g. nullability: FILTER can empty a group), pin the
  minimal failing case as an explicit test before fixing.

**Implementation shape.** Extend the aggregate strategy in
`tests/prop_helpers/generators.rs` with an optional FILTER wrapper reusing the existing
predicate generator (`generate_having_predicate`-style). Nullability note: an aggregate
with FILTER behaves like the aggregate over a possibly-empty group — verify the
nullability oracle agrees.

**Critical files.**
- `crates/smelt-db/tests/prop_helpers/generators.rs` (+ inference/divergence files only on triaged failures).

**Docs touched.** None (test coverage).

**Review checklist:**
- [ ] Smoke test proves reachability; 500-case run clean or failures pinned+triaged
- [ ] Nullability oracle re-run (shared generators)

**Commit.** `test(props): generate aggregate FILTER clauses against the DuckDB oracle`

### Phase 2: ordered-set aggregates + WITHIN GROUP

**Goal.** MEDIAN, MODE, PERCENTILE_CONT, PERCENTILE_DISC generated in DuckDB's syntax
(the historical deferral was "DuckDB syntax differences" — DuckDB accepts direct-arg
forms like `median(x)`, `percentile_cont(0.5 ORDER BY x)` / WITHIN GROUP variants; probe
the oracle for the exact accepted forms first). `STRING_AGG … WITHIN GROUP (ORDER BY …)`
generated if the parser already accepts it (TODO says parsed-not-generated).

**Pre-conditions.** Phase 1 pattern established.

**TDD tests to write first.**
- Oracle probe tests pinning DuckDB's accepted syntax + result types for each function
  (esp. `median` over INTEGER → DOUBLE vs DECIMAL — a `median_decimal` divergence is
  already registered; extend, don't duplicate).
- Reachability smoke tests; pinned regressions for any inference failure.

**Implementation shape.** New generator arm for ordered-set aggregates; registry entries
if any of the four are unrecognised (registry-first, ratchet must not grow). Reuse the
existing `median_decimal` divergence entry family where Spark disagrees.

**Critical files.**
- `generators.rs`, `crates/smelt-types/src/signatures.rs` (if unregistered), `divergences.rs` (evidence-backed only).

**Docs touched.** None.

**Review checklist:**
- [ ] Syntax oracle-probed, not assumed; 500-case run clean or pinned
- [ ] Registry gates green; ratchet unchanged

**Commit.** `test(props): generate ordered-set aggregates (MEDIAN/MODE/PERCENTILE_*) and WITHIN GROUP`

### Phase 3: two-column aggregates

**Goal.** CORR, COVAR_POP, COVAR_SAMP, REGR_SLOPE generated (deferral was "needs
multi-column aggregate generator support").

**Pre-conditions.** None.

**TDD tests to write first.**
- Reachability smoke test; oracle-pinned return types (all → DOUBLE on DuckDB — verify).

**Implementation shape.** Generator arm picking two numeric columns (the mixed-type
binary-op generator already selects column pairs — reuse that selection helper).
Register the four in `BuiltinRegistry` if missing (aggregate classification).

**Critical files.**
- `generators.rs`, `signatures.rs` (if needed).

**Docs touched.** None.

**Review checklist:**
- [ ] Registry consistency green; 500-case run clean or pinned

**Commit.** `test(props): generate two-column aggregates (CORR/COVAR/REGR_SLOPE)`

### Phase 4: ARRAY types in generators

**Goal.** TODO §Types: ARRAY literals (`[1, 2, 3]`), `ARRAY_AGG(x)`, subscript
(`arr[1]`), slice (`arr[1:2]`) generated; `BaseType`/Arrow mapping extended for
`Array<T>` element types.

**Pre-conditions.** None.

**TDD tests to write first.**
- Arrow-mapping unit tests: DuckDB `LIST` → smelt `Array<T>` for each base element type.
- Reachability smoke tests per construct; pinned regressions for inference failures
  (subscript result nullability — out-of-bounds yields NULL — is a likely first catch;
  verify with the nullability oracle).

**Implementation shape.** New `ExprKind` arms (ArrayLiteral, ArraySubscript, ArraySlice)
+ ARRAY_AGG in the aggregate pool; element-type-aware generation so subscripts type as
`T` and slices as `Array<T>`. `arrow_mapping.rs` already claims Duration/Interval
handling — extend the List branch analogously.

**Critical files.**
- `generators.rs`, `arrow_mapping.rs`, inference files on triaged failures.

**Docs touched.** None.

**Review checklist:**
- [ ] Element-type round-trip exact (no blanket Unknown); any Unknown matches `known_unknowns.rs`
- [ ] Both property oracles re-run

**Commit.** `test(props): generate ARRAY literals, ARRAY_AGG, subscript and slice`

### Phase 5: ROW/STRUCT constructors in generators

**Goal.** `ROW(...)` / `STRUCT(...)` / `{'k': v}` constructors generated with
struct-typed comparison against DuckDB's schema.

**Pre-conditions.** Phase 4 (nested-type comparison plumbing).

**TDD tests to write first.**
- Arrow-mapping tests: DuckDB `STRUCT` → `DataType::Struct` field-exact.
- Reachability smoke tests; pinned regressions.

**Implementation shape.** Generator arm building small fixed-arity structs from existing
typed columns; comparison path must recurse into fields (extend `type_comparison.rs` if
it is scalar-only today).

**Critical files.**
- `generators.rs`, `arrow_mapping.rs`, `type_comparison.rs`.

**Docs touched.** None.

**Review checklist:**
- [ ] Field-level exactness (names + types); oracles green at 500 cases

**Commit.** `test(props): generate ROW/STRUCT constructors with field-exact comparison`

### Phase 6: consolidation I — analysis modules single-sourced in smelt-logical

**Goal.** Delete `smelt-planner`'s parallel copies of `analysis/{mod,source_bounds,temporal}.rs`,
`logical.rs`, `types.rs`; `smelt-planner` consumes the `smelt-logical` versions
(finishing the extraction per ROADMAP §"smelt-logical / smelt-planner extraction" and the
layered single-ownership invariant).

**Pre-conditions.** T1 Phase 9 (its temporal.rs fix must land in the surviving copy —
verify it applies to the `smelt-logical` copy, and that the planner copy being deleted
doesn't carry divergent behaviour the planner depends on).

**TDD tests to write first.**
- A divergence audit *test-first* step: for each duplicated module, diff the two copies;
  any behavioural divergence gets a pinning test against the intended (smelt-logical)
  behaviour before the switch. If the copies diverge materially, STOP and mark blocked
  with the diff summary — do not silently pick a side.
- Existing planner + logical suites are the regression net.

**Implementation shape.** Re-export/import swap in `smelt-planner` (`use smelt_logical::…`),
delete the local copies, fix visibility. No behaviour change intended.

**Critical files.**
- `crates/smelt-planner/src/{analysis/*,logical.rs,types.rs}` (deletions + import rewires), `crates/smelt-logical` visibility only.

**Docs touched.** `docs/specs/architecture.md` — only if its crate-responsibility table
still describes the duplicated state (timeless wording).

**Review checklist:**
- [ ] `cargo tree -p smelt-db -i smelt-planner` still shows no production path
- [ ] Divergence audit recorded in the commit body (identical / pinned differences)
- [ ] `cargo test -p smelt-planner -p smelt-logical` green; full verify-phase green

**Commit.** `refactor(planner): consume smelt-logical analysis modules, delete duplicated copies (part 1)`

### Phase 7: consolidation II — rules, graph, lowering

**Goal.** Same treatment for `rules/{incremental,cumulative,rule_diagnostics,cube_split}.rs`,
`graph.rs`, `lowering/as_struct.rs`, leaving `smelt-planner` only its planner-only pieces
(`logical_plan_rules.rs`, `plan_printer.rs`, `python_bridge.rs`).

**Pre-conditions.** Phase 6.

**TDD tests to write first.**
- Same divergence-audit-first protocol as Phase 6.
- Rule *application* stays in `smelt-planner` (the invariant): a structural check that
  `smelt-logical` gains no dependency on planner application types.

**Implementation shape.** As Phase 6. Note `detect_builtin_rules` and rule-data
classifiers belong in `smelt-logical`, application in `smelt-planner` — the split is
already specified in CLAUDE.md §Layered single-ownership; this phase makes the file
layout match it.

**Critical files.**
- `crates/smelt-planner/src/{rules/*,graph.rs,lowering/as_struct.rs}`, import rewires.

**Docs touched.** `docs/specs/architecture.md` crate table if stale.

**Review checklist:**
- [ ] `cargo tree` assertion green; rule application still planner-side
- [ ] Full verify-phase green; statement_parity + walk_coverage gates green

**Commit.** `refactor(planner): consume smelt-logical rules/graph/lowering, delete duplicated copies (part 2)`

### Phase 8: cold-Salsa benchmark regression — profile and report

**Goal.** Root-cause the 2000-model `initial_load_ms`/`full_diagnostics_ms` blow-up
(~14.8s vs 10s ceiling, red on `main` since 2026-07-09; suspected: the monotonicity
trace / composition walk / per-model logical analyses from the PR #151-era merges).
**Deliverable is a written finding, not necessarily a fix**: if a single unambiguous
hot-spot with an obviously-safe fix emerges (e.g. a memoization gap or accidental
O(n²)), fix it; otherwise record the profile breakdown + options under master decision
D-QG-5 and mark this phase done-with-report.

**Pre-conditions.** None (independent).

**TDD tests to write first.** None up-front (investigation). If a fix is made: a
benchmark-adjacent regression assertion or unit test pinning the eliminated redundancy,
plus before/after numbers in the commit body.

**Implementation shape.** Run
`cargo run --release -p smelt-bench --bin profile_initial_load` against
`examples/huge/`; attribute time to the new analyses (walk, monotonicity trace, skew
classifier); write findings into this plan file under "## Bench findings" (dated) and
mirror the decision ask into the master's D-QG-5 row.

**Critical files.**
- `crates/smelt-bench/` (read/run); production code ONLY for an unambiguous safe fix.

**Docs touched.** None.

**Review checklist:**
- [ ] Findings section written with numbers per analysis pass
- [ ] Any fix is behaviour-preserving (all gates green) with before/after timings
- [ ] No ceiling raise (that is D-QG-5, a human decision)

**Commit.** `perf(bench): profile cold-Salsa 2000-model regression; findings + [fix|decision brief]`

### Phase 9: CLI ergonomics — `--verbose` output + `smelt test` skip diagnostic

**Goal.** (a) `smelt build --verbose` actually displays compiled SQL as advertised
(today: no extra output). (b) `smelt test` emits a "discovered but skipped" notice for
`materialization: test` files with a boolean-SELECT body instead of silently skipping.

**Pre-conditions.** None.

**TDD tests to write first.**
- CLI test: `smelt build --verbose` on a small example workspace includes compiled SQL
  for at least one model (assert on a distinctive substring).
- CLI test: `smelt test` over a fixture containing a skipped test file prints a notice
  naming the file; exit code unchanged.

**Implementation shape.** (a) wire the existing verbose flag to print each
`CompiledModel`'s SQL via the reporter (respect run-pipeline parity — output goes through
`execute_project`'s reporter, not ad hoc println; user-facing stdout in smelt-cli is the
allowed surface). (b) add the notice at the discovery/skip site in the test runner.

**Critical files.**
- `crates/smelt-cli/src/` (verbose wiring, test-runner notice), `crates/smelt-runtime/` reporter surface if needed.

**Docs touched.** `docs-site` CLI reference for `--verbose` semantics; `docs/specs/cli.md`
if it documents the flag (timeless wording).

**Review checklist:**
- [ ] Run-pipeline parity respected (no `pub` widening of smelt-runtime internals)
- [ ] `println!` gate unaffected (smelt-cli stdout is the allowed surface)
- [ ] execute_parity gate green

**Commit.** `fix(cli): make build --verbose show compiled SQL; smelt test reports skipped test files`

---

## Blocked phases

(Append dated entries here; never stop-the-line.)

## Bench findings

(Phase 8 writes its dated profile breakdown here.)

## Deferred during implementation

(Append-only.)

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` and
  `--test nullability_property_tests prop_nullability_sound` clean
- `cargo tree -p smelt-db -i smelt-planner` — no production path
- `cargo test -p smelt-runtime --test execute_parity` and `--test statement_parity` green
