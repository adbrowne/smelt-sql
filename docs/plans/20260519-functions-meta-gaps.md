# Plan: Functions + Meta-Language Implementation Gaps

**Date**: 2026-05-19
**Specs**:
- [`docs/specs/functions.md`](../specs/functions.md)
- [`docs/specs/meta_language.md`](../specs/meta_language.md)
- [`docs/specs/scoping.md`](../specs/scoping.md)
- [`docs/specs/types.md`](../specs/types.md)
- [`docs/specs/gradual_typing.md`](../specs/gradual_typing.md)
- [`docs/specs/planner_integration.md`](../specs/planner_integration.md)

**Spec diff**: none directly — this plan addresses **implementation gaps against existing specs**. Each phase moves a "Known Divergences" entry to "no longer divergent" and removes the entry as part of its commit. Phases 13–15 are spec-edit phases for the three open design decisions still tagged in spec bodies.

**Tracking PR / branch**: TBD — open PR when Phase 1 lands.
**Docs**: code+docs. Every phase that closes a gap also updates `docs-site/` and the relevant spec's "Known Divergences" section.

**Predecessor plans (closed):**
- [`docs/plans/20260422-smelt-functions.md`](20260422-smelt-functions.md) — 58 phases, complete 2026-04-27.
- [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md) — Phases A–G, complete 2026-05-18.

This plan picks up where those left off: the items those plans deliberately deferred, plus the two blockers found while implementing `examples/web_analytics/` (see Phases 4–8 of [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md), specifically the function-file comments in `examples/web_analytics/functions/parse_event_payload.sql` and `examples/web_analytics/functions/sessionize.sql`).

## Goal

Close the documented implementation gaps in the smelt functions and meta-language surfaces. Anchored by:

1. **The web_analytics blockers.** `examples/web_analytics/` declares `parse_event_payload` and `sessionize` as canonical `smelt.define` signatures but the models inline equivalent SQL because the call-site expander cannot yet (a) substitute named arguments or (b) ergonomically project struct-literal return values into model SELECT lists. Phase 1 fixes both.
2. **Phase 57 deferred tests** of `20260422-smelt-functions.md` — FROM-position aliasing, Spark struct-literal lowering, literal-VALUES models.
3. **The "Known Divergences" sections** of the active specs — runtime evaluation gaps, LSP wiring, type-system polish, built-in registry coverage.
4. **The open design decisions** still tagged in spec bodies (AmbiguousColumn code, per-ModelDef frontmatter, Array<U> constructor, path-component lift).

Each phase below has its own per-phase plan generated via `/smelt:plan <feature>` at the time the phase is picked up. Per-phase plans cite the relevant spec section(s) and run the standard `/smelt:implement` loop (implementer + reviewer subagents).

## How to resume in a fresh session

Standard smelt-plan flow (mirrors `20260517-web-analytics-example.md`):

1. Read this file (the in-repo phase status table).
2. Find the first non-`done` phase below.
3. If a per-phase plan exists (`docs/plans/20260519-functions-meta-<N>-<slug>.md`), read it. Otherwise generate it: run `/smelt:spec <feature>` only for spec-edit phases (15–18); for impl phases (1–14), run `/smelt:plan` with the phase scope as input.
4. Run `/smelt:implement docs/plans/20260519-functions-meta-<N>-<slug>.md`.
5. Run the per-phase plan's "Expert reviewer dispatch loop" until each expert reports no material findings.
6. Verification gates: `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`. For spec-edit phases: `/smelt:validate <feature>` zero drift.
7. Update the row below: `pending` → `done`, fill Date and Commit. Push. Remove the corresponding entry from the spec's "Known Divergences" section in the same commit.
8. End the session — the next iteration resumes from the next pending row.

**Subagent model rule.** Outer orchestrator runs on `opus`. Every delegated subagent — implementer, reviewer, every expert in per-phase reviewer tables — MUST be spawned with `model: "sonnet"`. Do not omit the `model` parameter.

## Phase status

| # | Phase | Status | Plan path | Date | Commit |
|---|-------|--------|-----------|------|--------|
| 1 | Function call expansion in model contexts: named-arg substitution + struct-returning function lowering (the web_analytics blocker) | pending | TBD | | |
| 2 | TableExpr inlining in FROM position: synthesize derived-table aliases | pending | TBD | | |
| 3 | Cross-engine codegen: Spark struct-literal lowering + literal-VALUES models (Phase 57 deferred tests) | pending | TBD | | |
| 4 | `joins:` / `provenance:` cardinality-enum mapping + validator tightening | pending | TBD | | |
| 5 | `joins:` / `provenance:` L2 planner-rule prototype (2 representative rules) | pending | TBD | | |
| 6 | Meta-language runtime: `smelt.sources.with_tag()` and source-reflection generator-body evaluation | pending | TBD | | |
| 7 | Meta-language: `ColumnRef.type` returns `DataType` literal (not `Unknown`) | pending | TBD | | |
| 8 | Meta-language: lift-scope validation wired at expansion time | pending | TBD | | |
| 9 | LSP Backend dispatch: record/map/loader hover, completion, goto-def, and diagnostic emission | pending | TBD | | |
| 10 | LSP Backend dispatch: lifted-identifier hover + goto-def; ModelRef/SourceRef goto-def at splice sites | pending | TBD | | |
| 11 | LSP: Tier 1 / Tier 3 return-type hover; multi-level frame trace rendering polish | pending | TBD | | |
| 12 | Type system: precision-aware Decimal arithmetic; `Float` collapsed to `Double` per spec | pending | TBD | | |
| 13 | Built-in registry coverage expansion: operators, missing aggregates/windows, JSON family, CAST | pending | TBD | | |
| 14 | Diagnostic polish: mint `AmbiguousColumn` code; tighten `fragment_param_kinds` corner cases | pending | TBD | | |
| 15 | Open design: `Array<U>(...)` runtime-array constructor — `/smelt:spec` decision | pending | TBD | | |
| 16 | Open design: per-`ModelDef` frontmatter override — `/smelt:spec` decision | pending | TBD | | |
| 17 | Open design: path-component identifier lift (`smelt.sources.<e.source_table>`) — `/smelt:spec` decision | pending | TBD | | |
| 18 | Open design: `smelt.as_struct` finalisation + `Expr<Struct<{…, ..r}>>` row polymorphism — `/smelt:spec` decision | pending | TBD | | |

## In scope (per-phase detail)

### Phase 1 — Function call expansion in model contexts (the web_analytics blocker)

**Spec sections**: `functions.md` §"Known Divergences" — *"End-to-end `smelt build` execution of `smelt.<path>(...)` function calls is incomplete"*. Predecessor: `docs/plans/20260422-smelt-functions.md` Phase 57 deferred test `e2e_passing_clause_substitution_executes`.

**Concrete blockers (from `examples/web_analytics/`):**

- **Named-argument substitution.** `SmeltFnExpander` substitutes positional parameters only; named-arg bindings (`partition_col => device_id, ts_col => event_ts, ...`) are emitted verbatim to the backend. Confirms in `examples/web_analytics/functions/sessionize.sql` comment block: *"column-reference arguments to smelt functions in model contexts are not yet supported"*. Model `silver/sessions.sql` inlines the equivalent `SUM(CASE WHEN ts_gap > gap OR platform_change THEN 1 ELSE 0 END) OVER (...)` window SQL instead.
- **Struct-returning function lowering.** `parse_event_payload(payload) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>>` is declared but `silver/events_parsed.sql` inlines `json_extract_string(payload, '$.event_name') AS event_name` three times. Comment block: *"smelt's struct-returning function expansion in model contexts is not yet ergonomic"*. The expander needs to project struct fields when the call site is `f(...).field` or `f(...).*` in a SELECT list.

**Acceptance:**
- `examples/web_analytics/models/silver/events_parsed.sql` rewritten to call `smelt.functions.parse_event_payload(payload)` and project the struct fields. Output schema unchanged. Diagnostics gate (`cargo test -p smelt-cli --test example_diagnostics`) passes with zero findings on the example.
- `examples/web_analytics/models/silver/sessions.sql` rewritten to call `smelt.functions.sessionize(source => smelt.ref('events_parsed'), partition_col => device_id, ts_col => event_ts, platform_col => platform)`. Output schema unchanged. Diagnostics gate passes.
- New e2e test in `crates/smelt-cli/tests/` runs `smelt build` on `examples/web_analytics/` after the refactor and asserts row counts + column values match the pre-refactor snapshot.
- Phase 57 deferred test `e2e_passing_clause_substitution_executes` enabled and passing.

**Spec edits in this commit:**
- Narrow the *"End-to-end `smelt build` execution of `smelt.<path>(...)` function calls is incomplete"* entry in `functions.md` Known Divergences to the remaining cross-engine cases (handled in Phase 3) and remove it entirely once Phase 3 lands.
- Add an example of struct-returning function projection to the functions guide in `docs-site/`.

### Phase 2 — TableExpr inlining in FROM position requires derived-table aliases

**Spec sections**: `functions.md` §"Known Divergences"; predecessor `20260422-smelt-functions.md` Phase 56 deferral note (line 1274).

**Gap**: When a `TableExpr`-returning function is inlined into a `FROM` slot, the substituted result is `FROM (SELECT ...)`, which DuckDB requires to carry an alias. Two options: (a) synthesize a unique alias at the lowering layer (transparent to the user; recommended), or (b) make the diagnostic-time check require an explicit `AS x` at the call site.

**Acceptance**: A model SQL with `FROM smelt.functions.sessionize(...)` (no alias) compiles and executes on DuckDB. Snapshot test added.

### Phase 3 — Cross-engine codegen: Spark struct-literal + literal-VALUES models

**Spec sections**: `functions.md` §"Known Divergences"; predecessor `20260422-smelt-functions.md` Phase 57 deferred tests.

**Gap**: Two deferred Phase 57 test cases:
- `e2e_spark_struct_literal_lowering_executes` — `{f: v}` lowers to `struct(v AS f)` on Spark.
- `e2e_literal_values_model_executes` — literal-VALUES models inside `smelt.fn.*` bodies execute correctly across engines.

**Acceptance**: Both tests pass; the "incomplete" entry in `functions.md` Known Divergences is fully removed. Spark CI already runs three jobs in `.github/workflows/compat.yml` (`Parse Equivalence Tests (PG + Spark)`, `Spark SQL Integration Tests (Docker)`, `Type Property Tests (DuckDB + Spark)`) — the new test cases plug into the existing Docker-based harness.

### Phase 4 — `joins:` / `provenance:` cardinality-enum mapping + validator tightening

**Spec sections**: `planner_integration.md` §"Known Divergences"; `functions.md` §"Known Divergences" — *"`joins:` and `provenance:` parsing is partial"*. Predecessor: `20260422-smelt-functions.md` Phase 51.

**Gap**:
- `JoinSpec.cardinality` currently stores the raw string (`"1:1"`, `"1:N"`, `"N:M"`). Phase 51 deferred the mapping to a `Cardinality` enum.
- Validator edge cases (`ProvenanceMismatch`, `JoinsMismatch`) are wired for basic shape but corner cases (e.g. transitive provenance through a chain of transparent functions; ambiguous cardinality when a join condition has multiple key columns) lack explicit test coverage.

**Acceptance**:
- `Cardinality` enum minted (likely in `crates/smelt-core/` alongside other planner metadata). Variants for `OneToOne`, `OneToMany`, `ManyToOne`, `ManyToMany`.
- `JoinSpec.cardinality` parses + stores the enum variant; invalid strings emit a parse-time diagnostic with a "expected one of: 1:1, 1:N, N:1, N:M" hint.
- New broken-fixture tests for ambiguous-cardinality and transitive-provenance cases. Existing tests continue to pass.
- Spec entry in `functions.md` Known Divergences narrowed to remove the cardinality bullet.

### Phase 5 — `joins:` / `provenance:` L2 planner-rule prototype

**Spec sections**: `planner_integration.md` §"Known Divergences"; depends on Phase 4. Predecessor: `20260422-smelt-functions.md` Phases 32–34 (planner rule API at Level 1).

**Gap**: Only two L1 rules consume `joins:` / `provenance:` (`PushFilterIntoTransparentFunction`, `EliminateUnusedLeftJoin`). L2 is where **strategy selection** happens — choosing between equivalent rewrites based on declared function properties. No L2 rule exists yet.

**Scope**: This phase is a **bounded prototype**, not a full L2 expansion. Pick 2 representative L2 rules that exercise distinct uses of declared metadata and demonstrate the planner-rule API at Level 2 works end-to-end. Suggested candidates:
1. **Push aggregation through transparent function** — when `joins:cardinality` declares row-preservation, agg pushdown is safe.
2. **Swap join order based on declared provenance** — when `provenance:` declares which side a column originates, the planner can reorder joins to minimise intermediate rows.

Concrete rule selection is finalised in the per-phase plan.

**Acceptance**:
- Both rules implemented end-to-end behind the `unstable_schema:` gate.
- Each rule has a fixture under `examples/` (or `crates/smelt-planner/tests/`) showing the optimisation firing — the unoptimised plan is captured via `smelt compile --show-plan` and compared to the optimised plan.
- A property test ensures correctness preservation: rule firing produces results equal to rule not firing (snapshot equivalence on a non-trivial fixture).
- Spec entry in `planner_integration.md` Known Divergences updated to cite the specific rules that landed; not removed (more L2 rules will follow, but the API is now demonstrated).

### Phase 6 — Meta-language runtime: source-reflection generator-body evaluation

**Spec sections**: `meta_language.md` §"Known Divergences" — *"Generator-body driver expansion incomplete"*.

**Gap**: `smelt.sources.with_tag()` and other source-reflection drivers type-check but `evaluate_body_emissions` doesn't iterate over them at expansion time. Only `smelt.config.load_yaml/json` drivers evaluate. `examples/staging_from_sources/` uses a hardcoded list-literal workaround.

**Acceptance**:
- `evaluate_body_emissions` iterates `smelt.sources.with_tag(...)` results and emits one model per matched source.
- `examples/staging_from_sources/` rewritten to use the idiomatic `smelt.sources.with_tag(...) | map { ... smelt.define_model ... }` form. Diagnostics + build gates pass.

### Phase 7 — `ColumnRef.type` returns `DataType` literal, not `Unknown`

**Spec sections**: `meta_language.md` §"Known Divergences".

**Gap**: `c.type == Integer` evaluates `Unknown == Integer` → never true → filter predicates silently produce empty list. Type-checker recognises the field but maps result to `Unknown`.

**Acceptance**:
- `c.type` returns a concrete `DataType` meta-literal.
- `examples/meta_columns/` exercises a HOF filter on `c.type == Integer` and produces non-empty results. Snapshot test.

### Phase 8 — Lift-scope validation at expansion time

**Spec sections**: `meta_language.md` §"Known Divergences".

**Gap**: A lifted column name that doesn't exist in the call-site schema currently produces incorrect SQL silently. Need expansion-time validation that the lift target is reachable.

**Acceptance**:
- New broken-example fixture under `examples/per_cohort_union_broken_lifted_column_unknown/` emits a diagnostic at the lift site.
- All currently-passing valid lift sites continue to compile.

### Phase 9 — LSP: record/map/loader Backend dispatch + diagnostic emission

**Spec sections**: `meta_language.md` §"Known Divergences" — *"Record/Map/loader LSP Backend dispatch not wired"*, *"Record/Map diagnostic codes not emitted via file_diagnostics"*.

**Gap**: Pure helpers exist for hover/completion/goto-def on `smelt.record`, record literals, map method calls, and loader sites; the `Backend::hover/completion/goto_definition` dispatchers don't route through them. `check_file_diagnostics` doesn't walk `RECORD_LITERAL` / `MAP_METHOD_CALL` nodes; the 10 record codes and 7 map codes don't surface as LSP squiggles.

**Acceptance**:
- LSP integration test on `examples/meta_config/` asserts hover/completion/goto-def behaviour for records, maps, and loaders.
- All record + map diagnostic codes emit through `file_diagnostics`. Broken-fixture tests assert they fire.

### Phase 10 — LSP: lifted-identifier + ModelRef/SourceRef goto-def

**Spec sections**: `meta_language.md` §"Known Divergences" — *"Lifted-identifier hover/goto-def Backend dispatch not wired"*, *"`ModelRef`/`SourceRef` goto-def at splice sites is graceful no-op"*.

**Gap**: Pure helpers + file-path-aware Salsa queries needed in Backend layer. Backend doesn't detect cursor inside lift positions or on `ModelRef`/`SourceRef` values at splice sites.

**Acceptance**:
- LSP hover on a lifted column-ref / AS-alias / ORDER BY / GROUP BY position shows the expansion-time identifier.
- Goto-def on a `ModelRef`/`SourceRef` at a splice site navigates to the `.sql` or YAML.

### Phase 11 — LSP polish: Tier 1/3 hover + multi-level frame rendering

**Spec sections**: `gradual_typing.md` §"Known Divergences" — *"LSP hover for Tier 1 return types"*, *"Multi-level frame rendering is deferred"*; predecessor `20260422-smelt-functions.md` Phases 24 / 29 / 48 / 50.

**Gap**:
- Pure helper `declared_return_hover_text(sig)` exists but the LSP `hover()` handler doesn't call it on `smelt.fn.*` call sites.
- Multi-level frame trace data is on disk; renderer reads only outermost call + innermost error. Intermediate "in expansion of B" line missing for nested chains (A → B → C).

**Acceptance**: LSP integration tests assert the rendered hover string + the multi-level frame trace string.

### Phase 12 — Type system: Decimal precision + Float collapsed to Double

**Spec sections**: `types.md` §"Known Divergences" — *"Decimal arithmetic v1 fallback"* and *"Promotion chain implementation drift (Float handling)"*.

**Gap**:
- `Decimal(19,2) + Decimal(19,2) → Decimal(38,10)` (DuckDB native: `Decimal(19,2)`). `ABS(Decimal) → Double` (pre-existing).
- Implementation orders chain as `... < Float < Decimal < Double`; spec normative is `Float` collapsed to `Double`.

**Acceptance**: Type-property tests aligned with DuckDB's precision rules pass. Spec entries removed.

### Phase 13 — Built-in registry coverage expansion

**Spec sections**: `functions.md` §"Known Divergences"; predecessor `20260422-smelt-functions.md` Phase 50.

**Gap**: Registry missing: `LIKE`, `ILIKE`, `IS NULL`, `BETWEEN`, `IN`, `EXISTS` (operators); `STRING_AGG`, `ARRAY_AGG`, `MEDIAN`, `STDDEV`, `VARIANCE` (aggregates); `FIRST_VALUE`, `LAST_VALUE`, `NTILE` (windows); `DATE_ADD`; full `JSON_*` family; `CAST`. Legacy `match` fallback at `crates/smelt-db/src/type_inference.rs:541` remains.

**Acceptance**: All listed functions routed through the canonical registry. Legacy fallback in `type_inference.rs` deleted. Property tests pass.

### Phase 14 — Diagnostic polish

**Spec sections**: `scoping.md` §"Known Divergences" — *"Ambiguous bare-column references"*, *"`fragment_param_kinds` seeding"*.

**Gap**:
- Ambiguous bare-column references currently surface as `UnknownIdentifier` with a hint. Mint a dedicated `AmbiguousColumn` code with a "available in N tables: ..." hint.
- `SelectItems<Agg>` referenced inside a non-aggregate splice (and analogous fragment-param-kind corner cases) need explicit test coverage.

**Acceptance**: New broken-example fixtures for both diagnostics. Spec entries removed.

### Phases 15–18 — Open design decisions

These are **spec-edit phases**, not impl phases. Per `feedback_namespace_falls_out_of_structure.md`, defer implementation until concrete pain emerges; reserve the slot to track each gap. Each phase:

1. Runs `/smelt:spec <feature>` to clarify the decision in the spec.
2. If the spec edit commits to implementation, follows with `/smelt:plan` and a per-phase impl plan.
3. If pain is still abstract, the phase closes by re-tagging the Known Divergence as "open design — revisit when X surfaces" and removing it from this plan.

- **15.** `Array<U>(...)` runtime-array constructor (`meta_language.md` Known Divergences).
- **16.** Per-`ModelDef` frontmatter override — multi-emission generators currently can't vary `cluster_by`/`partition_by`/`owner` per emission (`meta_language.md` Known Divergences).
- **17.** Path-component identifier lift `smelt.sources.<e.source_table>` — currently restricted to four SQL-expression positions; expand to path components if a killer demo surfaces (`meta_language.md` Known Divergences).
- **18.** `smelt.as_struct` finalisation + `Expr<Struct<{…, ..r}>>` row polymorphism (`functions.md` Known Divergences; research §16 #19). Strategy 3 of the no-overlap rule is "design sketch, available but not finalised" in v1; Strategies 1 (CTE rename) and 2 (typed `TableExpr<{…}>`) are recommended. Decide whether Phase 1's struct-returning lowering creates concrete pressure to finalise now, or whether the design can stay deferred. If pain is concrete (e.g. a new example requires multi-table struct namespacing), the phase commits the spec edit and opens an impl plan tracking row-polymorphic struct return types end-to-end. If still abstract, re-tag with the specific pressure that would trigger revisit ("multi-table function design with column-name collisions across joined sources").

## Out of scope

- **Adding new spec features.** Every phase closes a documented gap; no new surface area is introduced.
- **Rewriting `20260422-smelt-functions.md` or `20260509-meta-language-overall.md` history.** Those plans are frozen at their completion dates per `feedback_plans_historical.md` — no edits.
- **Full L2/L3 planner-rule expansion beyond Phase 5's prototype.** Phase 5 ships 2 representative L2 rules to demonstrate the API works end-to-end. Broader L2/L3 work (saturating the rule set, L3 strategy selection across model boundaries) follows under a separate plan once the prototype validates the shape.

## Verification

The plan is **done** when:

- All impl phases (1–14) show `done` above.
- The "Known Divergences" sections of `functions.md`, `meta_language.md`, `scoping.md`, `types.md`, `gradual_typing.md`, `planner_integration.md` no longer reference items closed by phases 1–14.
- `/smelt:validate functions`, `/smelt:validate meta_language`, `/smelt:validate planner_integration` report zero drift on closed entries.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all pass.
- Spark CI (`compat.yml` jobs `Parse Equivalence`, `Spark SQL Integration`, `Type Property DuckDB + Spark`) continues to pass after Phase 3.
- `examples/web_analytics/models/silver/{events_parsed,sessions}.sql` call `parse_event_payload` and `sessionize` directly (no inlined workarounds). Snapshot e2e test passes.
- Phases 15–18 are either resolved (spec committed to implementation and phase impl plan landed) or formally re-tagged as "open design, revisit when X" in their respective specs.
