# Plan: Split materialization into three axes (kind / storage / refresh)

**Date**: 2026-06-26
**Spec**: [`docs/specs/models.md`](../specs/models.md) (anchor) + [`testing.md`](../specs/testing.md), [`cumulative_aggregate.md`](../specs/cumulative_aggregate.md), [`architecture.md`](../specs/architecture.md), [`diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: commit `397a87ed` (this branch) — "spec: split materialization into three axes"
**Tracking PR / branch**: `impl/materialization-three-axes` (placeholder — create off `main` before Phase 1)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/models.md` §"Three orthogonal axes" / §"Refresh axis", `docs/specs/testing.md` (Surface + Semantics), and `docs/specs/cumulative_aggregate.md` Surface — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `impl/materialization-three-axes`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Salsa purity, fail-loud discipline, layered single-ownership — `smelt-db` has no production dep on `smelt-planner`; cumulative classifier data lives in `smelt-logical`).
- Build/test with system DuckDB (`DUCKDB_LIB_DIR` set); use `cargo test --quiet 2>&1 | tail -40` per the token-efficiency rules in `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. The specs are already updated by `397a87ed`; further spec edits during implementation (e.g. recording a gap) describe behaviour, not phases. `docs-site/docs/...` edits read as if the feature has always existed.

---

## Context

The user surface conflated three independent questions under `materialization` (`models.md` §"Three orthogonal axes"): a test (kind) and a cumulative aggregate (refresh) were both materialization *values*. The engine already separates these (`EntityKind::Test`; the backend `Materialization` is storage-only — Table/View/MaterializedView). This plan drives the user surface to match: `materialization` becomes storage-only, a unit test becomes a `smelt.test` declaration (`testing.md`), and cumulative becomes `refresh: cumulative` on a table (`cumulative_aggregate.md`, the sibling of `incremental`).

## Scope

### In scope (spec coverage)
- `models.md` §"Refresh axis" — new `refresh:` frontmatter key and `RefreshStrategy {Full, Cumulative}`.
- `cumulative_aggregate.md` Surface — cumulative opt-in moves from `materialization: cumulative_aggregate` to `materialization: table` + `refresh: cumulative`; classifier/validation/diagnostics re-keyed; `Materialization::CumulativeAggregate` removed.
- `testing.md` Surface + Semantics — `smelt.test <name> AS (<select>) PASSING <dep> AS (<rows>) EXPECT (<rows>)` grammar; test-local `#` CTE operator; runner consumes the grammar (mock CTEs from `PASSING`, `EXPECT` comparison, `#cte` targeting); `check_order`/`cases` stay frontmatter; `materialization: test` and `TestConfig.{model,target_cte,inputs,expect}` removed.
- `architecture.md` §"Kind" — `smelt.test` recognised as a keyword-signalled kind by the parser (not string-match in the resolver).
- `diagnostics.md` — `UnknownTestInput` re-keyed to `PASSING`; new `UnknownTestCte`, `CteRefOutsideTest`.
- `cli.md` / `data_catalog.md` — `smelt explain --json` and catalog `materialization` enum narrowed; `refresh` field added.
- Example + crate-test migration to the new surface.

### Explicitly deferred
- **C-full refresh unification** — folding the `incremental:` block under a unified `refresh:` key. C-min keeps `incremental:` as-is; `refresh:` only carries `full`/`cumulative` (`models.md` §"Refresh axis").
- **Project-wide `#` CTE addressing** — `#` stays test-local (`testing.md` Known Divergences). No general cross-model CTE references.
- **Meta-language `ModelDef.materialization`** — `ModelDefInvalidMaterialization` still lists `{view, table, incremental}` and conflates storage+refresh in the generator surface (`meta_language.md`). Separate follow-up; not touched here.
- **`refresh:` precedence list** in `models.md` (frontmatter vs `smelt.yml`) — frontmatter-wins is implemented; pinning the precedence table is a doc-only follow-up.
- **Spark refresh/test paths** — unchanged; tests still run on DuckDB (`testing.md` §"Tests always use DuckDB").

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 7f202f67 | 2026-06-26 |
| 2     | done     | 3f9ea474 | 2026-06-26 |
| 3     | done     | 776ffd15 | 2026-06-26 |
| 4     | done     | 8d46c64d | 2026-06-26 |
| 5     | done     |        | 2026-06-27 |
| 6     | pending  |        |      |

---

### Phase 1: Introduce the refresh axis

**Goal.** Add the `refresh:` frontmatter key and a `RefreshStrategy {Full, Cumulative}` enum, and make the cumulative classifier/validation/execution recognise `materialization: table` + `refresh: cumulative` — identically to today's `materialization: cumulative_aggregate`. Both surfaces work after this phase (the old one is removed in Phase 2).

**Pre-conditions.** None — first phase.

**TDD tests to write first.**
- `crates/smelt-core/tests/...::refresh_cumulative_parses` — `materialization: table` + `refresh: cumulative` frontmatter deserializes to `RefreshStrategy::Cumulative`; a model with no `refresh:` key is `Full`.
- `crates/smelt-core/tests/...::refresh_cumulative_forbids_timeseries_and_incremental` — `refresh: cumulative` + `timeseries:` → `CumulativeForbidsTimeseries`; + `incremental:` → `CumulativeForbidsIncremental` (re-keyed off refresh, not materialization).
- `crates/smelt-core/tests/...::refresh_on_view_is_warning` — `view` + `refresh: cumulative` emits the ignored-config warning (mirrors `view` + `incremental`).
- `crates/smelt-cli/tests/cumulative_equivalence.rs::refresh_cumulative_matches_legacy` — a fixture written as `materialization: table` + `refresh: cumulative` produces the same execution/result as the equivalent `materialization: cumulative_aggregate` fixture (real-fixture: a new `examples/`-style cumulative model or an in-test workspace).

**Implementation shape.**
- `crates/smelt-core/src/config.rs` — add `RefreshStrategy { Full, Cumulative }` with `Deserialize`/`Serialize` (`"full"`/`"cumulative"`, default `Full`).
- `crates/smelt-core/src/metadata.rs` — add `refresh: Option<RefreshStrategy>` to `ModelMetadata`; move the cumulative forbid-`timeseries:`/forbid-`incremental:` validation (currently keyed on `Materialization::CumulativeAggregate`, lines ~363/373/398) to fire on `refresh == Cumulative`. Keep the old keying too until Phase 2 (transitional OR).
- Add a single predicate `ModelMetadata::is_cumulative()` returning true for `refresh: cumulative` **or** (transitionally) `materialization: cumulative_aggregate`. Route every current `Materialization::CumulativeAggregate` detection site (`smelt-runtime/src/execute.rs:208,701`, `smelt-db/src/lib.rs:1464`) through this predicate so Phase 2 only has to drop one arm.
- `crates/smelt-logical/src/rules/cumulative.rs` — classifier is already pure SQL classification; only its trigger condition changes (via the predicate above). No logic moves out of `smelt-logical`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy`, validation arm.
- `crates/smelt-core/src/metadata.rs` — `refresh:` field, `is_cumulative()`, re-keyed validation.
- `crates/smelt-runtime/src/execute.rs` — route cumulative dispatch through `is_cumulative()`.
- `crates/smelt-db/src/lib.rs` — cumulative classification via the predicate.

**Docs touched.**
- `docs-site/docs/guide/materializations.md` — document the refresh axis and `refresh: cumulative` (alongside `incremental`), written as existing feature.

**Review checklist.**
- [ ] TDD tests exist and assert refresh parsing + re-keyed diagnostics.
- [ ] `cumulative_aggregate.md` Surface rules satisfied by the `refresh: cumulative` path.
- [ ] Layered single-ownership honored — no classifier data logic leaves `smelt-logical`.
- [ ] No scope creep: `Materialization::CumulativeAggregate` is **not** removed yet.
- [ ] User docs updated to match Surface.
- [ ] Spec/docs-site edits are timeless.

**Commit.** `feat(core): add refresh axis (refresh: cumulative) alongside materialization`

---

### Phase 2: Cut cumulative over to refresh-only

**Goal.** Remove `Materialization::CumulativeAggregate`; `refresh: cumulative` is the only cumulative opt-in. Migrate all cumulative fixtures/examples and update explain/catalog output.

**Pre-conditions.** Phase 1 (refresh axis works; detection routed through `is_cumulative()`).

**TDD tests to write first.**
- `crates/smelt-core/tests/...::cumulative_aggregate_materialization_rejected` — `materialization: cumulative_aggregate` now fails to deserialize (unknown value) with a clear error.
- `crates/smelt-cli/tests/cumulative_diagnostics.rs` — update existing assertions to the `refresh: cumulative` surface; diagnostics unchanged in code, triggered by refresh.
- `crates/smelt-cli/src/commands/explain.rs` test (or `smelt-cli/tests/`) — `smelt explain --json` emits `"materialization": "table"` + `"refresh": "cumulative"` for a cumulative model (no `"cumulative_aggregate"` materialization string).
- `cargo test -p smelt-cli --test example_diagnostics` green after example migration.

**Implementation shape.**
- Remove `Test`? No — only `CumulativeAggregate` here. Drop the variant from `config.rs` enum + de/serialize; delete the transitional OR in `is_cumulative()`; delete now-dead `Materialization::CumulativeAggregate` match arms (`config.rs:850–877` cumulative portion, `execute.rs:987/990` unreachable arm, `smelt-db/src/lib.rs:1464`).
- **Also drop the transitional `|| materialization == Materialization::CumulativeAggregate` fallback clauses** that Phase 1 added alongside `is_cumulative()` at the detection sites — `execute.rs` (2 sites, ~210 and ~712) and `explain.rs` (~326). These exist in Phase 1 so smelt.yml-configured legacy models (where frontmatter `metadata` is `None`) still resolve; once the variant is gone they are dead and must be removed too. (Surfaced by the Phase 1 reviewer — dropping the `is_cumulative()` arm alone is not sufficient.)
- `explain.rs` / catalog (`docs.rs`) — serialize storage `materialization` + a `refresh` field (`"cumulative"`, omitted when full), per `cli.md` / `data_catalog.md` JSON schema.
- Migrate the 5 example files (`examples/cumulative_classifier_gate/models/edges_valid.sql`, `edges_bad_aggregator.sql`, `examples/timeseries_broken_cumulative_with_incremental/...`, `examples/timeseries_broken_cumulative_with_timeseries/...`, `examples/web_analytics/models/silver/device_user_edges.sql` + its README) and the cumulative crate fixtures (`backbuild_cumulative_e2e.rs`, `cumulative_equivalence.rs`, `cumulative_diagnostics.rs`) from `materialization: cumulative_aggregate` → `materialization: table` + `refresh: cumulative`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs`, `metadata.rs` — drop variant + transitional OR.
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-db/src/lib.rs` — remove dead arms.
- `crates/smelt-cli/src/commands/explain.rs`, `docs.rs` — JSON `refresh` field.
- `examples/**` (cumulative) + `crates/smelt-cli/tests/cumulative_*.rs`, `backbuild_cumulative_e2e.rs`.

**Docs touched.**
- `docs-site/docs/guide/materializations.md` — ensure no `materialization: cumulative_aggregate` remains; cumulative shown only as `refresh: cumulative`.

**Review checklist.**
- [ ] `Materialization::CumulativeAggregate` fully removed; no dead arms remain (compiler-enforced exhaustiveness clean).
- [ ] `cargo test -p smelt-cli --test example_diagnostics` + `example_workspaces` green.
- [ ] explain/catalog JSON matches `cli.md`/`data_catalog.md` schema.
- [ ] No scope creep into the test/kind axis.
- [ ] User docs contain no `materialization: cumulative_aggregate`.

**Commit.** `refactor!: remove materialization: cumulative_aggregate in favor of refresh: cumulative`

---

### Phase 3: Parser — `smelt.test` declaration grammar

**Goal.** Parse `smelt.test <name> AS (<select>) [PASSING <dep> AS (<rows>)]... EXPECT (<rows>)` into a first-class AST node, mirroring `smelt.define`. No semantic wiring yet.

**Pre-conditions.** None (parser is independent of Phases 1–2).

**TDD tests to write first.**
- `crates/smelt-parser/tests/...::parse_smelt_test_basic` — a `smelt.test` declaration with a SELECT body, one `PASSING` clause, and an `EXPECT` clause parses into a `SmeltTest` node with the expected children; lossless round-trip.
- `crates/smelt-parser/tests/...::parse_smelt_test_multiple_passing` — zero and many `PASSING` clauses.
- `crates/smelt-parser/tests/...::expect_rows_are_record_literals` — `EXPECT ( {a: 1, b: 'x'} , {a: 2} )` parses as record literals (omitted keys allowed — property-test shape).
- `crates/smelt-parser/tests/...::passing_expect_contextual` — `PASSING`/`EXPECT` remain ordinary identifiers outside a `smelt.test` (column alias, CTE name) — no regression to `functions.md` PASSING positional rule.

**Implementation shape.**
- `crates/smelt-parser/src/syntax_kind.rs` — `SMELT_TEST`, `EXPECT_CLAUSE` node kinds (PASSING_CLAUSE exists).
- `crates/smelt-parser/src/parser/smelt_ext.rs` — add `at_smelt_test_trigger()` (copy `at_smelt_define_trigger`, match `"test"`); a `parse_smelt_test()` that parses `<name>`, `AS ( <select> )`, reuses `parse_passing_clause()`, then a new `parse_expect_clause()` (contextual `EXPECT` then `( <rows> )` where rows are record-literal lists). Register the trigger in `parse_file()`.
- `crates/smelt-parser/src/ast.rs` — `SmeltTest` typed wrapper (name, body select, passing clauses, expect clause) following `SmeltDefine`.

**Critical files.**
- `crates/smelt-parser/src/syntax_kind.rs`, `parser/smelt_ext.rs`, `ast.rs`.

**Docs touched.** Code-only at the parser layer (the user-facing grammar is documented in `testing.md`, already updated). No docs-site change this phase.

**Review checklist.**
- [ ] TDD parser tests exist; lossless round-trip holds.
- [ ] `EXPECT`/`PASSING` contextual-keyword rule from `functions.md` preserved.
- [ ] Error recovery: a malformed `smelt.test` produces a parse error, not a panic.
- [ ] No semantic/runner changes leaked into this phase.

**Commit.** `feat(parser): parse smelt.test declarations (AS / PASSING / EXPECT)`

---

### Phase 4: Parser — `#` CTE-reference operator (test-local)

**Goal.** Lex/parse `smelt.<path>#<cte>` as a CTE reference, and emit `CteRefOutsideTest` when it appears outside a `smelt.test` body.

**Pre-conditions.** Phase 3 (`SmeltTest` node exists to scope "inside a test body").

**TDD tests to write first.**
- `crates/smelt-parser/tests/...::parse_hash_cte_ref` — `FROM smelt.daily_revenue#daily_agg` parses as a `smelt.<path>` reference carrying a `#`-suffixed CTE segment.
- `crates/smelt-db/tests/...::cte_ref_outside_test_diagnostic` — a `#` CTE ref in a model body (not a `smelt.test`) yields `CteRefOutsideTest`, anchored at the `#`.
- `crates/smelt-db/tests/...::cte_ref_inside_test_ok` — the same ref inside a `smelt.test` body produces no `CteRefOutsideTest`.

**Implementation shape.**
- `crates/smelt-parser/src/lexer.rs` + `syntax_kind.rs` — add a `HASH` token; admit it in the `smelt.<path>` reference parser to capture a trailing `#<ident>`.
- AST — expose the optional CTE segment on the smelt-path reference wrapper.
- `crates/smelt-db/src/lib.rs` (diagnostics path) — `CteRefOutsideTest` when a `#`-ref's enclosing declaration is not a `SmeltTest`. (`UnknownTestCte` — does the named CTE exist — is checked in Phase 5 where the model-under-test is resolved.)

**Critical files.**
- `crates/smelt-parser/src/lexer.rs`, `syntax_kind.rs`, `ast.rs`.
- `crates/smelt-db/src/lib.rs` — `CteRefOutsideTest` emission + `map_metadata_error_to_diagnostic` exhaustiveness if a new variant is added.

**Docs touched.** Code-only at parser/diagnostics layer (`testing.md` §"CTE references" already documents `#`; `diagnostics.md` already lists the code).

**Review checklist.**
- [ ] `#` token + ref parsing tests green; lossless round-trip.
- [ ] `CteRefOutsideTest` fires only outside `smelt.test` bodies.
- [ ] Fail-loud: no silent acceptance of a stray `#` ref.
- [ ] No runner wiring in this phase (`UnknownTestCte`/execution is Phase 5).

**Commit.** `feat(parser): test-local # CTE-reference operator + CteRefOutsideTest`

---

### Phase 5: Test runner on the new `smelt.test` kind

**Goal.** Drive `smelt test` from the parsed `smelt.test` AST: assertion query body, `PASSING` mock CTEs keyed by dependency address, `EXPECT` comparison, `#cte` targeting, with `UnknownTestInput`/`UnknownTestCte` diagnostics and `check_order`/`cases` from frontmatter. The legacy `materialization: test` path **coexists** so existing examples stay green until Phase 6.

**Pre-conditions.** Phases 3–4 (grammar + `#`).

**TDD tests to write first.**
- `crates/smelt-cli/tests/...::smelt_test_full_query_passes` — a new-syntax `smelt.test` over a real `examples/` model: `PASSING` mocks the dep, `EXPECT` matches; PASS. Wrong `EXPECT` → FAIL.
- `crates/smelt-cli/tests/...::smelt_test_cte_isolation` — a `#cte` test runs the target CTE's chain as written, mocking only external deps; correct rows pass.
- `crates/smelt-cli/tests/...::passing_unknown_dep_diagnoses` — a `PASSING` name matching no dependency → `UnknownTestInput` (loud fail, not a false green).
- `crates/smelt-cli/tests/...::hash_unknown_cte_diagnoses` — `#nonexistent` → `UnknownTestCte`.
- `crates/smelt-cli/tests/...::check_order_and_cases_from_frontmatter` — `check_order: true` enforces positional compare; omitted `EXPECT` columns trigger the property loop with `cases` iterations (CTE-targeted).

**Implementation shape.**
- `crates/smelt-core/src/resolver.rs` — `classify_sql` recognises a parsed `smelt.test` (replace the bare `content.contains("smelt.test")` heuristic with the real AST kind once available); keep `materialization: test` → `EntityKind::Test` transitionally.
- `crates/smelt-cli/src/test_compiler.rs` + `test_runner.rs` — new entry points consuming the AST: build mock CTEs from `PASSING` rows (reuse the YAML→SQL coercion table, now applied to record-literal values), resolve `#cte` to the target CTE + its internal chain, mock external deps, compare actual vs `EXPECT` per `testing.md` §"Comparison behavior". Dispatch in `commands/test.rs`: a `smelt.test` declaration uses the new path; a legacy `materialization: test` model uses the existing path.
- Diagnostics — `UnknownTestInput` re-keyed to `PASSING`; add `UnknownTestCte` resolution (model-under-test's `WITH` lacks the named CTE).

**Critical files.**
- `crates/smelt-cli/src/test_compiler.rs`, `test_runner.rs`, `commands/test.rs`.
- `crates/smelt-core/src/resolver.rs` — AST-based kind classification.
- `crates/smelt-db/src/lib.rs` — `UnknownTestInput`/`UnknownTestCte` wiring.

**Docs touched.**
- `docs-site/docs/guide/testing.md` — document the `smelt.test` grammar, `PASSING`/`EXPECT`, and `#cte` isolation as the testing surface (written as existing feature).

**Review checklist.**
- [ ] All runner TDD tests green against real `examples/` fixtures.
- [ ] `testing.md` Semantics (full-query, CTE-level, property-based, comparison) satisfied.
- [ ] Legacy `materialization: test` path still works (coexistence) — existing example tests green.
- [ ] Salsa purity: diagnostic analysis stays pure; queries are thin wrappers.
- [ ] User docs (testing guide) match Surface.

**Commit.** `feat(cli): run smelt.test declarations (PASSING/EXPECT/#cte) end to end`

---

### Phase 6: Cut tests over to the new kind; remove `materialization: test`

**Goal.** Migrate every test to the `smelt.test` grammar, remove `Materialization::Test` and the moved `TestConfig` fields (`model`/`target_cte`/`inputs`/`expect`; keep `check_order`/`cases`), and route all kind-filtering through `EntityKind::Test`.

**Pre-conditions.** Phase 5 (new runner path proven).

**TDD tests to write first.**
- `crates/smelt-core/tests/...::materialization_test_rejected` — `materialization: test` now fails to deserialize (unknown value).
- `crates/smelt-cli/tests/...` (the ~12 existing `materialization: test` fixtures) — rewritten to `smelt.test` syntax, still asserting the same pass/fail outcomes.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` green after example migration.
- `crates/smelt-cli/tests/...::explain_and_catalog_exclude_tests` — `smelt explain`/catalog still exclude `smelt.test` declarations (now via `EntityKind::Test`, not `is_test()` on materialization).

**Implementation shape.**
- `crates/smelt-core/src/config.rs` — remove `Materialization::Test` from enum + de/serialize; remove the `Test` validation arm (`config.rs:850–877` test portion).
- `crates/smelt-core/src/metadata.rs` — drop `TestConfig.{model,target_cte,inputs,expect}`; `TestConfig` retains `check_order`/`cases` as the frontmatter knob struct.
- `crates/smelt-core/src/discovery.rs` — `is_test()` reads `EntityKind::Test` (parsed `smelt.test`), not `materialization == Test`.
- Filtering sites — `run_setup.rs`, `explain.rs`, `docs.rs`, `smelt-db/src/queries/project.rs:439`, `smelt-ui/src/build.rs:40`, `smelt-cli/src/executor.rs:25` — route through the AST kind; remove the now-dead `Materialization::Test` arms.
- Migrate ~17 example test files (`examples/**/tests/*.sql`, `examples/ephemeral_demo/models/*`, `examples/broken/models/failing_*`, `examples/meta_hofs/models/and_all_predicates.sql`, `examples/per_cohort_union/tests/...`) and remaining crate fixtures (`smelt-db/tests/model_frontmatter_diagnostics.rs`, `smelt-lsp/tests/diagnostics.rs`, `smelt-runtime/tests/select_parity.rs`, the `smelt-cli/tests/test_*.rs`).

**Critical files.**
- `crates/smelt-core/src/config.rs`, `metadata.rs`, `discovery.rs`, `resolver.rs`.
- Filtering sites listed above across `smelt-cli`, `smelt-db`, `smelt-ui`.
- `examples/**` test files + crate test fixtures.

**Docs touched.**
- `docs-site/docs/guide/testing.md` — final pass: no `materialization: test` anywhere; only `smelt.test`.

**Review checklist.**
- [ ] `Materialization::Test` fully removed; enum is exactly storage modes (Table/View/Ephemeral/MaterializedView); `map_metadata_error_to_diagnostic` exhaustiveness clean.
- [ ] Every example + crate fixture migrated; `example_diagnostics` + `example_workspaces` green.
- [ ] explain/catalog/run all exclude tests via `EntityKind::Test`.
- [ ] No `materialization: test` remains in `examples/`, `crates/`, or `docs-site/`.
- [ ] User docs (testing guide) timeless and `smelt.test`-only.

**Commit.** `refactor!: remove materialization: test; tests are smelt.test declarations`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test --quiet 2>&1 | tail -40` — full workspace green.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — examples clean via both the Salsa-direct and real-LSP paths.
- `cargo clippy --all-targets` — no warnings.
- `rg -n "materialization: test|materialization: cumulative_aggregate" examples/ crates/ docs-site/` — zero hits.
- A real cumulative example builds with `materialization: table` + `refresh: cumulative`; a real `smelt.test` (full-query and `#cte`) passes via `smelt test`.
- `/smelt:validate testing`, `/smelt:validate models`, `/smelt:validate cumulative_aggregate` — zero drift.
