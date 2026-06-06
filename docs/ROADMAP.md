# smelt Development Roadmap

This document summarizes where each area of smelt stands and what's next. For detailed implementation plans, see [`docs/plans/`](plans/). For the canonical behavior of a feature, see [`docs/specs/`](specs/) — specs are the source of truth and plans cite them.

The **What's Next** section below is the prioritized work queue. Component sections that follow provide context on current state and per-area backlog items.

## Process

This project uses a spec-driven workflow. The flow:

1. `/smelt:spec <feature>` — capture or update `docs/specs/<feature>.md` (the canonical answer to "how does this feature work?")
2. `/smelt:plan <feature>` — derive a phased plan from the spec diff; plan cites the spec rather than restating it
3. `/smelt:implement <plan>` — per-phase implementer + reviewer subagents, red-green TDD on real fixtures, atomic commits
4. `/smelt:validate <feature>` — drift report comparing spec, code, and user docs

The mandatory plan structure (execution prompt, per-phase TDD tests, implementer/reviewer loop, code+docs phases by default) is encoded in `/smelt:plan`. See `CLAUDE.md` § Workflow & Slash Commands for the workflow overview.

## What's Next

The items below are the current priority queue, top to bottom. See completed items in [Recently Completed](#recently-completed) below.

The spine of the near-term roadmap is a single through-thread: **finish the shared runtime and the bug sweep → cover the missing type-system axes → build virtual environments on that precision → generalise to schema migration.** Spark hardening runs as an elevated parallel track; the remaining items are lower priority.

### 1. CLI Execute-Loop Migration to `smelt-runtime` (in-flight)

A two-plan architectural refactor consolidating CLI and UI on a shared compile + execute pipeline (`smelt-runtime`) per the Run Pipeline Parity Rule (`docs/specs/architecture.md`).

**Shipped** ([plan](plans/20260523-smelt-runtime-extraction.md)): `smelt-runtime` crate; `SqlCompiler` + emitters; `select_executable_models`; `execute_project`; cumulative dispatch. UI's `run_manager.rs` shrank 726→317 lines and now correctly expands `smelt.fn.*`, applies type casts, inlines ephemerals, filters tests, and runs cumulative models — closing four silent compile divergences.

**Next** ([plan](plans/20260524-cli-runtime-migration.md)): seven phases that finish the migration:

1. `compute_incremental_windows` + per-source bound machinery into runtime (bound-aware batch planner).
2. Planner safety check + temporal bound derivation + schema-evolution gate into runtime; `ExecuteRequest` gains the `allow_*` flags.
3. Eliminate `LogicalGraph` (884 lines) and `PhysicalGraph` (1184 lines); runtime returns `PlanSummary` for `--show-plan`.
4. CLI `commands/run.rs` migrates to `execute_project`; `StdoutReporter` + `model_compiled` callback for `--verbose`.
5. End-to-end CLI ↔ UI parity CI gate (`cargo test -p smelt-runtime --test execute_parity`).
6. `pub(crate)` lockdown of compile internals — half-compile becomes a type error.
7. Delete `smelt-cli`'s shim modules; spec lands the structural-enforcement clause.

Explicit non-goals: `smelt backbuild` migration (separate command, own plan); `smelt-language-service` extraction (separate plan, awaits UI editor work).

### 2. Finish the Feature Sweep / Bug Ledger

The autonomy loop drives a two-level backlog — a top-level feature-sweep ledger (the master to-do list) and focused remediation sub-plans (see `CLAUDE.md` § Autonomy loop). The recent run of `BUG-*` closures in [Recently Completed](#recently-completed) is its output: cross-family arithmetic strictness, function-default self-containment, address-collision enforcement, codegen soundness, frontmatter parity, and diagnostic parity. Goal: drive the ledger to green — every "LSP-clean but unbuildable" defect and spec-vs-code drift the sweep surfaces is either fixed or recorded as a tracked divergence.

This is the correctness baseline the rest of the queue builds on, so it stays near the top and is finished before the new big-rock features begin.

Open structural residue worth calling out:
- **BUG-064** — `smelt-db → smelt-planner` layering inversion. The fix is now a committed plan ([`20260606-smelt-logical-extraction.md`](plans/20260606-smelt-logical-extraction.md), P1–P5): extract a new `smelt-logical` crate holding the logical Plan / `LogicalNode` model + rule cluster *beneath* both `smelt-db` and `smelt-planner` — superseding the earlier "move the queries into `smelt-planner`" idea.

### 3. Silent Failures & Code-Health Hardening

A recurring source of hard-to-diagnose bugs is failure that is *swallowed* rather than surfaced. The March 28 hardening pass (`unwrap()` audit, `tracing` migration) was a one-time sweep and has since regressed, so this item makes "fail loud, or handle it" an ongoing tracked discipline — high priority alongside the feature sweep (#2) because it is the same correctness baseline. Four fronts:

- **Silent `Unknown` / fallback-without-diagnostic (highest value).** Type inference and resolution collapse to `DataType::Unknown` in ~160 places, many emitting no diagnostic — the user-facing symptom is "inference is silently wrong, with no error." Worst confirmed case: `crates/smelt-types/src/signatures.rs:2200` turns an unparseable struct-field type into `Unknown` with no diagnostic, which then propagates downstream invisibly. The work: audit every `Unknown`/`None`/default sentinel on the inference + resolution paths and split *legitimate* Unknown (meta-language `Any`, deliberate conservatism) from *error* Unknown (parse failure, missing annotation, unresolved ref), emitting a diagnostic for the latter.
- **Swallowed errors.** `let _ =`, `.ok()`, `.ok()?` chains that drop error context, and no-op `Err(_) =>` arms — concentrated in the type-inference and LSP layers, where a parse/resolution *reason* is lost and the user sees only "type unknown." Triage the cases that hide a real, reportable failure.
- **Panics & `unwrap()` debt.** ~1,800 `unwrap()`/`expect()` in production project-wide (per the 2026-05-17 codebase review — *up* from the post-March-audit baseline; `crates/smelt-cli/src/python.rs` alone holds ~125), plus genuine input-validation `panic!`s in `smelt-datagen` and `smelt-state/ddl_spark` that should be recoverable errors. Re-run the audit, convert recoverable panics to errors, and add a CI ratchet (e.g. a `clippy::unwrap_used` budget) so the count cannot silently climb again. Also finish the stalled `println!`→`tracing` migration (~237 `println!` remain).
- **Divergent / duplicate implementations.** Logic implemented twice in parallel paths drifts apart. The largest case — the CLI vs `smelt-runtime` execute loops (schema-evolution checks, planner safety gate, `LogicalGraph`/`PhysicalGraph`) — is already being closed by #1; the *durable* fix is to land that plan's deferred `execute_parity` CI gate and the `pub(crate)` compile-internals lockdown, so future divergence becomes a compile error rather than a review catch. Remaining straggler to single-source: `build_fn_body_map` vs `build_fn_body_map_from_model_files` (default-extraction logic duplicated across the Salsa and non-Salsa paths). The single-source invariants that currently *hold* — workspace loading, frontmatter parsing, seed/function/source discovery, the SQL compiler, the diagnostic gate — should stay that way.

A fifth, lighter front — **surfacing deferred work that was never tracked** — lives in the [Deferred-Work Backlog](#deferred-work-backlog-untracked-follow-ups) section below: follow-ups left in `docs/plans/` that never reached the roadmap. The task there is to keep that catalogue current and triage items into the queue, not to resolve them here.

### 4. Type-System Axes — Decimal, Nullability, Collation, Timezone

smelt's type system tracks base types and NULL propagation structurally, but four axes are coarse or untracked. They are simultaneously (a) real-world correctness gaps and (b) the precision blocker for virtual environments — `output_fingerprint.md` lists decimal/collation/nullability among the untracked axes that force conservative rebuild. Covering them sharpens both, which is why they are sequenced immediately before Virtual Environments.

- **Decimal** — carry precision/scale through inference and arithmetic; widen division results correctly (today DECIMAL inference is too narrow and overflows past 99 — the residual from the smelt_shop validation).
- **Nullability** — tighten NULL tracking into an exact, fingerprint-grade property rather than an advisory one.
- **Collation** — model string comparison/sort semantics as a tracked attribute; required before collation-sensitive output can be declared equivalent.
- **Timezone** — distinguish timestamp-with / -without-timezone and make inference tz-aware.

As each axis lands, the fingerprint oracle gains real precision on it instead of falling back to verbatim rebuild.

### 5. Virtual Environments + Backbuild Change-Detection (specs authored, prototype proven)

SQLMesh-style opt-in virtual data environments: cheap isolated environments that share physical tables with production whenever a model's output is *provably* unchanged, rebuilding only what provably changed. The differentiator over SQLMesh is a **typed, provable equivalence relation** in place of a syntactic edit-script. The same machinery powers **backbuild change-detection** — deciding precisely which models a change forces to rebuild versus spares.

**Proven** (research + Stage 0 prototype): the semantic output-fingerprint oracle ([`crates/smelt-fingerprint`](../crates/smelt-fingerprint)) with its soundness gate (`fingerprint-equal ⇒ DuckDB relations identical`) and determinism detector, all green as property tests against DuckDB. See Recently Completed below and [`docs/research/20260601-virtual-environments.md`](research/20260601-virtual-environments.md).

**Specced**: [`output_fingerprint.md`](specs/output_fingerprint.md) (normative, implemented), [`virtual_environments.md`](specs/virtual_environments.md) (the orchestration layer — `state.mode`, environment addressing, fingerprint-keyed reuse, promotion, override hatches), [`run_state.md`](specs/run_state.md) (`.smelt/` layout + snapshot store).

**Next** (each increment gated by the DuckDB oracle, derived via `/smelt:plan`):
1. Wire `output_fingerprint` into the runtime (it is a standalone prototype today). Depends on the runtime consolidation in #1.
2. Snapshot store + `(environment, model) → table` map (`run_state.md`); fingerprint-keyed reuse for a single environment.
3. `state.mode: environments` addressing, `smelt plan/apply --environment`, `smelt promote`.
4. **Backbuild change-detection** — the cross-model column-lineage analyser computing the full "eclipse" (downstream models spared by an output-preserving upstream change); the gating new analysis, and the substrate item #6 builds on.
5. Polish: typed data-diff, GC/retention, forward-only.

Explicit non-goal for now: the un-annotated determinism inversion remains conservative-rebuild until covered (worst-case parity; see `output_fingerprint.md` Known Divergences). The type-system axes that previously forced conservative rebuild are addressed in #4 and unlock fingerprint precision as they land.

### 6. General Schema Migration on the VE Substrate

Generalise schema change management on top of the fingerprint + column-lineage machinery from #5. smelt already has schema evolution (ALTER vs full-refresh, complex/nested types) and offline `smelt diff`; this item makes migration planning lineage-aware, so a plan knows precisely which downstream models are output-affected versus spared (the same eclipse analysis), and can stage and preview migrations across environments before promotion. Sequenced after Virtual Environments because it reuses that substrate.

### 7. Spark — Production Hardening

The Spark backend is functionally complete (PySpark/PyO3 bridge, zero-copy Arrow, Spark Connect / Databricks Connect). Remaining gaps to production-grade:

- **Integration-test parity** — run the DuckDB integration suite against a local Spark Connect server, so Spark is verified at the same depth as DuckDB.
- **JSON incompatibility rewrites** — `TO_JSON(scalar)`, `JSON_CONTAINS`/`@>`/`<@`, `JSON_OBJECT`/`JSON_ARRAY`; emit compile-time warnings where no faithful rewrite exists.
- **Authentication docs** — tokens, OAuth, and instance profiles for Databricks Connect / EMR / Dataproc.

### 8. `smelt check` — LLM-Optimised Diagnostic CLI

Structured diagnostic output designed for LLM consumption. Exposes Smelt's semantic analysis (parse errors, type errors, resolution failures, schema compatibility) via `smelt check --format json` with severity filtering, file/project scope, token budget control (`--budget-lines`), and optional extended context (`--explain`). Replaces the previously planned `smelt validate`. Includes a Claude Code skill and eval harness for empirically tuning diagnostic sufficiency.

See [design doc](plans/20260405-smelt-check.md) for full interface spec, JSON schema, and eval plan.

### 9. Orchestrator Integration

Dagster/Airflow plugin API. `smelt explain --json` already provides the graph structure; next step is a thin adapter layer for orchestrator consumption.

### 10. PostgreSQL Backend

Third backend after DuckDB and Spark. Deprioritized earlier in favor of Spark, now the remaining major backend gap.

### 11. Databricks Support + Metrics-View Compatibility (low priority)

Deeper Databricks integration beyond the existing Spark / Databricks-Connect path, treated as low priority. The long-deferred **Metrics DSL** (`smelt.metric()`) is folded in here: Databricks now ships first-class **metrics views**, so the concrete, testable goal is that smelt metric definitions are compatible with — and can target — Databricks metrics views. That compatibility test is the forcing function that gives the Metrics DSL a real spec to hit; absent that, the Metrics DSL stays low priority and is tracked here rather than as its own item.

---

## Recently Completed

### ~~Per-Target Config Overlay — production wiring complete~~ ✅ (June 6, 2026)

Closed BUG-014: a headline `meta_config_loading` feature (`smelt build --target prod` reading a sibling `<basename>.prod.<ext>` overlay and merging it into the base) that was implemented and unit-tested at the loader layer but had zero production callers ([plan](plans/20260605-per-target-overlay-wiring.md)):

- **`active_target` Salsa input** (P1) — `Option<Arc<str>>` field on the singleton `Workspace` input in `crates/smelt-db/src/lib.rs`; `set_active_target` setter; retiring the lib.rs stub comment. Salsa invalidation verified.
- **Target threading through `load_workspace`** (P2) — `smelt-core::workspace::load_workspace` reads the `smelt.yml` `target:` field as the default; CLI `--target` overrides it on the build/run path. Overlay files (`cohorts.prod.yaml`) paired as overlay inputs rather than orphan base inputs in `workspace_ingest.rs`. Dual gate (`example_diagnostics` + `example_workspaces`) stays green — CLI↔LSP discovery symmetric.
- **`collect_loader_values` dispatches to overlay** (P3) — computes `<basename>.<target>.<ext>` from the effective target and calls `loader_resolved_value_with_overlay` when the overlay input exists; falls back to base-only resolution when target is unset or the overlay is absent. `examples/meta_config_overlay_probe`: `smelt build --target prod` now yields `revenue >= 999` (not the base `>= 100`).
- **Overlay validation diagnostics wired** (P4) — a schema-violating overlay surfaces `ConfigLoaderUnknownField` anchored at the overlay file's offending row and fails the build; same diagnostic family as a base-file mismatch. `examples/meta_config_overlay_probe_invalid` gate.
- **Close-out** (P5) — end-to-end regression test `meta_config_e2e.rs::e2e_per_target_overlay_wires_into_generator_build` confirms dev/prod row counts; spec References updated; BUG-014 flipped to fixed in ledger.

### ~~Cross-Family Arithmetic Strictness — `Unknown` + `TypeMismatch` enforced~~ ✅ (June 6, 2026)

Enforced the types spec rule that cross-family binary arithmetic (e.g. `42 + '3'`, `TRUE + 1`) must produce `Unknown` and emit a `TypeMismatch` diagnostic ([plan](plans/20260605-cross-family-arithmetic-strictness.md)):

- **Family guard added to `promote_numeric_operands`** (P1) — `crates/smelt-db/src/type_inference/binary.rs`: the catch-all `(Some(l), _) => Some(l)` arm now checks family compatibility before returning the left type; a cross-family pair yields `DataType::Unknown` instead of the left operand's type. Numeric/numeric promotion and `INTERVAL * numeric` special cases unchanged. Closed BUG-017 (1/2). Regression tests: `test_cross_family_arithmetic_numeric_plus_string`, `test_cross_family_arithmetic_boolean_plus_numeric`, `test_cross_family_arithmetic_numeric_plus_string_literal`.
- **`check_crossfamily_arithmetic_diagnostics` + `TypeMismatch` emission** (P2) — new walker in `binary.rs`; wired into `file_diagnostics` and the `check_types` query so a `TypeMismatch` Error is emitted at the operator span for each cross-family arithmetic operation. Closed BUG-017 (2/2). Keepable fixture `examples/types_broken_crossfamily_add/`. Regression tests: `file_diagnostics_emits_type_mismatch_crossfamily_numeric_plus_string`, `file_diagnostics_emits_type_mismatch_crossfamily_boolean_plus_numeric`, `types_broken_crossfamily_add_emits_type_mismatch`.
- **§296 Known Divergence narrowed** (P3) — `docs/specs/types.md` updated to clarify that cross-family binary arithmetic is now enforced while the composite-path gaps (array-literal / `UNION` unification) remain deferred.

### ~~Function Default Self-Containment — Semantics #9 enforced~~ ✅ (June 6, 2026)

Enforced the functions spec rule that a default expression must not reference other parameters in the same signature ([plan](plans/20260605-function-default-self-containment.md)):

- **`DefaultReferencesParameter` diagnostic code** — new variant added to `DiagnosticCode`; documented in `functions.md` §Diagnostic codes table. Closed BUG-003 (1/2).
- **AST-side validator** — near `default_type_lookup` in `crates/smelt-db/src/queries/function_diagnostics.rs`; for each parameter with a default, scans the default expr's identifier/column-ref tokens for a sibling-param-name match and emits `DefaultReferencesParameter` anchored at the default expr's range. Self-contained defaults (`= 1`) produce no diagnostic. Closed BUG-003 (2/2). Regression tests: `default_referencing_sibling_param_is_error`, `self_contained_default_is_ok` in `crates/smelt-db/tests/function_body_check.rs`.

### ~~smelt.yml Surface Alignment — spec drift closed, unknown-key warning~~ ✅ (June 5, 2026)

Closed spec-vs-code drift on the `smelt.yml` and CLI model-selection surfaces ([plan](plans/20260605-smelt-yml-surface-alignment.md)):

- **`backbuild` positional-selector documented** (P1) — `model_selection.md` §Flags amended: removed `backbuild` from the `--select` "Available on" cell; added a note that `backbuild` takes a single positional selector, always forces `+` upstream, and has no `--exclude`. Closed BUG-046 (spec-only).
- **`timeseries` row added to model-config table** (P1) — `smelt_yml.md` §"Model-config shape" now lists `timeseries | object | absent | Time-dimension declaration`. Closed BUG-059 (spec-only).
- **`cumulative_aggregate` added to `default_materialization` accepted values** (P1) — the Surface table and its `docs-site/` mirror now list all six valid values. Closed BUG-061 (spec-only).
- **Unknown top-level key warning implemented** (P2) — `Config::parse_with_warnings` now iterates the raw YAML map against a 10-key allow-list and emits a named warning per unknown key (including typos like `default_matrialization`). Legacy keys (`model_paths`, `seed_paths`) and `unstable_schema` are allow-listed to avoid duplicate/false-positive warnings. Closed BUG-060. Regression tests: `config::tests::unknown_top_level_key_warns`, `valid_config_with_all_known_keys_emits_no_generic_warnings`, `legacy_path_key_does_not_also_get_generic_unknown_key_warning`.

### ~~Address Collision Enforcement — workspace identity, discovery consolidation~~ ✅ (June 5, 2026) [BUG-002/021/040/063; P4 blocked]

Enforced the one-path-one-entity invariant across all entity kinds and consolidated seed discovery ([plan](plans/20260605-address-collision.md)):

- **Pure address-map resolver** (P1) — `smelt_core::resolve_address_map` computes the canonical `smelt.<path>` address for every discovered model (including within-file `--- name ---` sections), function, seed, and source; detects cross-kind and within-file collisions without a filesystem re-walk. Closed BUG-002 (1/2), BUG-021 (1/2). Regression test: `smelt-core::resolver::tests`.
- **`project_address_collisions` Salsa query** (P2) — surfaces `DuplicateAddress` (Error) diagnostics to CLI and LSP via the shared diagnostic channel; fixture `examples/architecture_broken_path_collision` (model `dup.sql` + seed `dup.csv`) confirmed. `smelt build` and `smelt explain --json` now exit non-zero on a collision. Closed BUG-002 (2/2). Regression tests: `crates/smelt-cli/tests/address_collision.rs`.
- **Model-name uniqueness surfaced** (P2b) — spec re-scoped uniqueness to canonical `smelt.<path>` address (Constraint 4 re-worded; stale cross-file example dropped); Python `ModelFile.address_segments` populated; `resolve_address_map` applied CLI-side over combined SQL+Python model set. Closed BUG-021 (2/2), BUG-040. Regression test: `test_python_model_name_collision` + `within_file_section_collision_surfaces_duplicate_address`.
- **Seed discovery consolidated** (P3) — `smelt-cli::discover_seeds` deleted; all seed lookups route through `smelt_core::discover_seed_infos_strict`; sources/functions audited (single-sourced). Eliminates the third seed-discovery path that was the structural root cause of the asymmetric-discovery bug family. Closed BUG-063.
- **P4 blocked** — BUG-064 (smelt-db → smelt-planner layering inversion) investigation complete: the leak is full planner logic (`detect_builtin_rules`, `parse_function_properties`, `collect_path_refs`), not a plain type. Options recorded in plan §"Blocked phases"; recommendation: move Salsa rule-diagnostic queries into `smelt-planner` (correct direction).

### ~~Codegen Soundness — CTE collisions diagnosed, `source.*` valid~~ ✅ (June 5, 2026)

Closed the silent-until-`smelt run` function-expansion defects found in the feature sweep ([plan](plans/20260604-codegen-soundness.md)):

- **`source.*` over a `smelt.<path>` argument now emits valid SQL** (C2) — the transparent-function splice aliases the argument to the parameter name (`FROM <arg> AS <param>`) instead of text-replacing `<param>` with the qualified name. `main.base.*` → `source.*` stays single-part and DuckDB-valid. Closed BUG-009. Regression fixture: `examples/fn_tableexpr_star/`.
- **CTE-collision diagnostic `CteShadowsCallerCte`** (C3) — new analysis-time check in `check_file_diagnostics`: when a model's top-level CTE name collides with a CTE in the body of a directly-called transparent function, the compiler emits `CteShadowsCallerCte` (Error) anchored at the call site, refusing the build rather than silently emitting wrong data. v1 covers direct collisions only; transitive collisions are a known gap (alpha-rename is v2). Closed BUG-007. Regression fixture: `examples/expansion_broken_cte_caller_collision/`.
- **`make_generator_frame` signature corrected in `expansion.md`** (C1) — doc-only retraction of the stale 3-arg form. Closed BUG-008.

### ~~Frontmatter Parity — unified catalogue, no silent drops~~ ✅ (June 4, 2026)

Collapsed two divergent frontmatter parsers into one over a key catalogue ([plan](plans/20260604-frontmatter-parity.md), [spec](specs/architecture.md) §"Unified frontmatter rule"):

- **`deny_unknown_fields` on `TimeseriesConfig`** (U1) — unknown `timeseries:` sub-keys now produce a serde error instead of being silently ignored. Closed BUG-025.
- **`FrontmatterCatalogue` + `parse_frontmatter`** (U2) — single entry point in `smelt-core::frontmatter`; catalogue maps each key to its applicable declaration kinds; unknown key → `Error`, inapplicable key → `Warning`, valid key → kept.
- **Model path wired** (U3) — `ModelMetadata` deserialized from the validated map; errors surfaced as `FrontmatterParseError`/`MalformedTimeseries` diagnostics in `file_diagnostics`. Closed BUG-016, BUG-023.
- **Function/extern path wired; second parser deleted** (U4) — `FunctionProperties` built via `parse_frontmatter`; hand-rolled `parse_function_properties` deleted. One parser remains.
- **E2E example fixtures + gates** (U5) — four regression examples: `frontmatter_function_key_on_model` (positive, builds as TABLE with Warning), `timeseries_broken_invalid_granularity`, `timeseries_broken_unknown_key`, `frontmatter_broken_unknown_key` (all build-refused with Error).
- **Deferred**: dynamic schema-registration API for non-built-in planner rules (tracked in [planner_rule_api_design.md](planner_rule_api_design.md)).

### ~~Diagnostic Parity (analysis ↔ build) + Meta-Language Codegen~~ ✅ (June 2026)

Closed the "LSP-clean but unbuildable" bug class surfaced by the feature sweep ([plan](plans/20260531-diagnostic-parity.md), [spec](specs/architecture.md) §"Diagnostic parity rule"):

- **Shared Error-severity build gate** (P2, June 1) — `smelt_runtime::gate_diagnostics` runs the full `file_diagnostics` surface (not just `UnknownSmeltFn`) before any model compiles; wired into both the CLI run path and `execute_project`. Closed BUG-015, 019, 024.
- **Uniform planner rule → diagnostics interface** (P2b, June 1) — cumulative classifier and incremental batch-safety/bounds checks now surface via `file_diagnostics` and are visible to both the editor and the build. Closed BUG-011.
- **Per-entity source diagnostics** (P2c, June 1) — new `project_source_diagnostics` Salsa query maps `SourceError` variants to `MalformedSource`/`SourceTypeError` diagnostics and publishes them to the LSP at init time. Closed BUG-032.
- **Nested `smelt.define` fixpoint** (P3, June 1) — printer's body-reparse now re-expands nested `SMELT_PATH_CALL` nodes to a fixpoint via a synthetic `SELECT`-prefix reparse; `functions_demo` nested-compose models execute correctly. Closed BUG-013.
- **Block `PASSING` fragment binding** (P4, June 2) — printer merges `PASSING <name> AS (<body>)` clauses into the existing named-arg vector before substitution; `rollup_with_passing` executes correctly. Closed BUG-018.
- **In-model meta-language at build** (P5–P7d, June 2–3) — a pure-text build-path meta evaluator in `smelt-runtime::meta_eval` lowers all analyzer-accepted constructs before codegen: list spread (P5), HOF/pipe/lambda/ternary/config.var (P6), `smelt.columns_of` reflection (P7a), wide reflection `smelt.models.*`/`smelt.sources.*` (P7b), bare List/Map loader detector + List-loader lowering (P7c), Map-loader via `MAP_METHOD_CALL` postfix parsing + `.keys()`/`.values()`/`.entries()` lowering (P7d). Closed BUG-006 (all sub-issues).
- **`example_builds` CI gate** (P1) — builds + executes every example workspace on DuckDB; `meta_config` removed from `KNOWN_UNBUILDABLE` after P7d; remaining entries are unseeded-source workspaces (structural, not codegen gaps).

### ~~Virtual Environments — research, Stage 0 prototype & specs~~ ✅ (June 1–4, 2026)

Proved the core thesis of opt-in virtual data environments — *reuse a physical table when a change is provably output-preserving* — without any state or environment machinery, then specced the feature set.

- **Semantic output-fingerprint oracle** ([`crates/smelt-fingerprint`](../crates/smelt-fingerprint), [spec](specs/output_fingerprint.md)): hashes a canonical normal form of a model's `SELECT` so two versions with the same fingerprint provably compute the same relation (multiset, columns by name). Recognises as equivalent — where SQLMesh's edit-script rebuilds — formatting, comments, keyword case, projection reorder, internal CTE/alias rename, and single-use-CTE ≡ derived-table (recursive sub-fingerprint). Conservative verbatim fallback everywhere else.
- **Soundness gate**: `fingerprint-equal ⇒ DuckDB relations identical` as a property test against DuckDB, with positive/negative golden corpora — the load-bearing invariant before any reuse is wired to execution.
- **Three soundness bugs found and fixed**, each via the discipline "generate the real-world shape and let DuckDB judge": implicit-alias column lists mis-parsed (`FROM (…) t(c1,c2)`, fixed on `main`); a derived-table-left **join** silently dropped by inlining; `LIMIT`/`OFFSET`/`QUALIFY` entirely absent from the canonical form (every top-N/paginated model collapsed to one fingerprint).
- **Determinism detector**: structural deny-list (non-deterministic built-ins, parenless temporal specials, order-sensitive aggregates) + row-slice-without-total-order check, surfaced as `deterministic` on the result. Gated so anything flagged deterministic reproduces across two independent DuckDB builds. Closes §5.5's value axes; window-function non-determinism is the noted residual.
- **Specs authored**: [`output_fingerprint.md`](specs/output_fingerprint.md) (normative), [`virtual_environments.md`](specs/virtual_environments.md) (staged orchestration design), [`run_state.md`](specs/run_state.md) (`.smelt/` layout); touched `architecture.md`, `incremental_models.md`, `schema_evolution.md`.

Research: [`docs/research/20260601-virtual-environments.md`](research/20260601-virtual-environments.md). Next: the implementation queue under [What's Next #5](#5-virtual-environments--backbuild-change-detection-specs-authored-prototype-proven).

### ~~Typed Meta-Language — Phase E2: Multi-Model Production~~ ✅ (May 16, 2026)

Completed Phase E2 of the typed meta-language plan ([plan](plans/20260509-meta-language-E2.md), [spec](specs/meta_language.md)):

- **`generates: models` frontmatter directive** — marks a file as a generator file whose body is a `List<ModelDef>` meta-expression. The `.gen.sql` extension is a recommended convention.
- **`ModelDef` built-in closed record type** — five fields: `name` (required, `Text`), `body` (required, `TableExpr`), `materialization` (optional, `Text`), `tags` (optional, `List<Text>`), `description` (optional, `Text`). User-constructible only inside generator file bodies.
- **W1–W4 workspace-shape resolution pipeline** (Salsa-cached): W1 discovers generator files; W2 evaluates each generator's body in isolation; W3 collision-checks and emits survivors/discarded; W4 type-checks the full workspace including emitted models.
- **Ten diagnostic codes**: `GeneratesUnknownValue`, `GeneratesMixedWithBareModel`, `GenerateFileBareSelectForbidden`, `GenerateFileBodyTypeError`, `ModelDefOutsideGeneratorFile`, `ModelDefInvalidName`, `ModelDefInvalidMaterialization`, `ModelDefDuplicateName`, `ModelDefHandAuthoredCollision`, `GeneratorBodyForbidsModelReflection`.
- **`<generator>` expansion frame** — `evaluate_generator` stamps the `<generator>` anonymous frame onto every diagnostic from inside the generator body's HOF chain. The frame has `function = "<generator>"`, `decl_path = generator_file_path`, `call_site_range = body expression range`.
- **Generator-file CLI integration** — `build_logical_graph` and `discover_emitted_model_files` in `smelt-cli` wire emitted models into the logical graph. `register_loader_files_from_disk` in `init_db` auto-registers YAML/JSON/TOML loader files so `smelt.config.load_yaml` calls in generator bodies can evaluate.
- **LSP pure helpers** — `hover_text_for_generates_frontmatter`, `hover_text_for_model_def_literal_open_brace`, `hover_text_for_model_def_name_field_value`, `hover_text_for_model_def_body_field_value`, `completion_for_generates_value`, `completion_for_model_def_field_key`, `goto_def_for_emitted_model_reference` — all unit-tested. Backend dispatch wiring is Phase G.
- **`examples/per_cohort_union/`** killer demo — three cohorts from `cohorts.yaml`, union in `all_cohorts_unioned.sql`, zero LSP diagnostics.
- **`examples/staging_from_sources/`** secondary demo — staging layer generator from source YAML files, zero LSP diagnostics.
- **Ten broken sub-fixtures** — one per diagnostic code under `examples/broken/meta_language_e2_broken/`.
- **User docs**: `docs-site/docs/meta-language/generators.md`, index/reflection/reference page additions.

See [plan](plans/20260509-meta-language-E2.md). Next: Phase G (rename, LSP completeness sweep, `/smelt-loop` `large` tier).

### ~~Smelt Functions — Steps 6–13 (PASSING, planner, struct row vars, review remediation)~~ ✅ (April 24–26, 2026)

Completed the remaining eight steps of the smelt-functions experimentation roadmap (Phases 28–53 of [plan](plans/20260422-smelt-functions.md)):

- **Step 6** (Phases 28–29, April 24): Context-sensitive `PASSING name AS (...)` parser (peek `PASSING` only after `smelt.fn.*` / user-defined call closings); binding PASSING fragments to `SelectItems` parameters with type-checking and kind-ceiling enforcement. `rollup_with_passing.sql` demo.
- **Step 7** (Phases 30–34, April 25): Functions as first-class `LogicalNode::FunctionCall` nodes in the logical plan; column provenance + declared-property propagation (`provenance:`, `joins:`, `deterministic:` frontmatter); `PlannerRule` trait + `apply_rules_to_fixed_point`; `ExpandTransparentFunctionCalls`, `PushFilterIntoTransparentFunction`, and `EliminateUnusedLeftJoin` rules. `--show-plan` CLI flag wired in Phase 39.
- **Step 8** (Phases 35–38, April 25): Struct row variables (`Struct<{..r}>`), value-level spread (`..event`), call-site row-var unification with erasure at expansion, `smelt.as_struct(<alias> EXCEPT ...)` with backend-specific struct-literal emission.
- **Steps 9–13 — review remediation** (Phases 39–53, April 25–26): 15 phases closing all 28 findings from the post-Phase-38 plan review. Key deliverables: `--show-plan` CLI integration (Phase 39), CAST emission from canonical-return registry (Phase 40), transparent-call body splice into logical plan (Phase 41), list-splice comma elision (Phase 41), `smelt.as_struct` lowering to `smelt-planner` + broadened capability gate (Phase 42), serde_yaml frontmatter parser replacing line-walker (Phase 43), `safe_divide` / `monitored_session_rollup` canonical fixtures (Phases 44–44b), JOIN alias visibility in `TableExpr` bodies + `enriched_order` workaround removed (Phase 45), `TableExpr` argument shapes extended to CTEs / derived tables / subqueries (Phase 46), cross-function CTE schema inference + opaque-CTE suppression dropped (Phase 47), LSP hover + PASSING completion + multi-level frame trace in message (Phase 48), `WindowInScalarContext` deep-walk into scalar subqueries (Phase 49), built-in registry expansion (operators, aggregates, window functions — Phase 50), `provenance:` / `joins:` validator (Phase 51), missing-provenance pushdown advisory + extern fragment-param rejection (Phase 52), plan audit / SHA table + cross-file extern collision fixture (Phase 53).

See [plan](plans/20260422-smelt-functions.md) for the full phase-by-phase record. User documentation: [Functions guide](../docs-site/docs/guide/functions.md).

### ~~Smelt Functions — Steps 1–5~~ ✅ (April 22–24, 2026)

Implemented the first five steps of the smelt-functions experimentation roadmap (Phases 1–27 of [plan](plans/20260422-smelt-functions.md)):

- **Step 1** (Phases 1–6, April 22): `smelt.define` / `smelt.fn.*` parser, Salsa signature index, `Expr<T>` type-reference resolution, Tier 1 body type-check, call-site expansion with single-level frame trace. `safe_divide` end-to-end demo. `examples/functions_demo/` workspace created and registered with CI.
- **Step 2** (Phases 7–12, April 23): `Ordered` constraint, canonical built-in signature registry (~40 functions, generics + variadics), `infer_function_type` rewired through registry, `smelt.extern` declarations, per-declaration frontmatter with `backends:` inference and backend-namespace sugar, multi-level frame rendering in LSP, CAST-enforcement flag on canonical returns.
- **Step 3** (Phases 13–18, April 23–24): `TableExpr` / `AggExpr` / `WindowExpr` / `SelectItems` type-ref grammar; `ExprKind { Scalar, Agg, Window }` with linear subtyping and `SelectItems<K>` kind ceiling; `TableExpr` bare-column row polymorphism with parameters-first scoping and shadow warnings; row-requirement annotations (`TableExpr<{col: Type, ..r}>`); `sessionize` end-to-end with TableExpr output-schema inference; LSP hover for `smelt.define` parameter types (`TableExpr<{...}}` and `Expr<...>` rendered); `add_margin → sessionize` pipeline fixture.
- **Step 4** (Phases 19–22, April 24): Context-binding parsing and resolution for `Expr<T, ctx>` and `SelectItems<Kind, ctx>`; CTE schema extraction (`extract_function_body_cte_schemas`) with topological ordering and opaque-CTE suppression for `SELECT * FROM smelt.fn.*` patterns; `unknown_context_diagnostics_for_file` extended to accept CTE names alongside parameter names; `check_fragment_context_bindings` extended to look up CTE column schemas; `()` empty-default parser support; `session_rollup` end-to-end demo added to `examples/functions_demo/`.
- **Step 5** (Phases 23–27, April 24): Tier 2 body check in isolation, Tier 3 return-type verification + LSP hover, call-site bidirectional pre-expansion checking (Phase 25), Tier 2 → Tier 1 inline expansion with frame-stack propagation (Phase 26), and bidirectional generics (`unify_call_with_expected` with `expected_return: Option<DataType>` propagated from `TypeContext`, Phase 27). Upgrade story documented in [`docs/smelt-functions-upgrade-story.md`](smelt-functions-upgrade-story.md).

**Deferred during Steps 1–5**: See "Deferred during implementation" appendix in the plan for the full list. Key items: structured `Synthesized` marker for default-value provenance, broad TableExpr argument shapes beyond `smelt.ref()`/`smelt.source()`, SQL comma-elision for empty `SelectItems` defaults (Phase 32/planner). PASSING clauses (Step 6), planner visibility (Step 7), struct row vars (Step 8).

### ~~Type Inference, Parser & Ref Resolution Fixes~~ ✅ (April 10, 2026)

All critical/major bugs from the smelt_shop real-world validation report fixed:

- **Seeds as `smelt.ref()` targets** — Seeds are now first-class dep-graph citizens. `resolve_ref()` searches seeds after SQL models; CSV column types inferred and provided to the type-checking layer. No more `sources.yml` workaround.
- **JOIN type inference** — Qualified column refs (`p.col`) no longer fall through to `infer_literal_type()`. Fixed by detecting dot patterns before decimal literal inference.
- **CASE expression column names** — `CAST(? AS TYPE) AS ?` bug fixed; compiler generates `_col1, _col2` deterministic names for unnamed CASE outputs.
- **CASE expression type widening** — `infer_case_expr_type` now promotes across all branches; `promote_types` widens Decimal+Integer to Decimal(38,10).
- **EXTRACT(EPOCH FROM ...)** — New dedicated `EXTRACT_EXPR` syntax kind in the parser handles `EXTRACT(field FROM expr)` without treating the FROM keyword as SQL FROM.
- **CTE type inference** — `parse_when_clause()` fixed to use `parse_or_expr()`, enabling full logical expressions in CASE WHEN.
- **Subquery ref replacement** — Subquery type inference now clones context and processes inner FROM before calling `infer_select_column_types`.
- **FLOAT→DOUBLE normalization** — `CAST(x AS FLOAT)` infers as DOUBLE; `float_division` and `cast_float_as_double` divergences documented.
- **Materialization type changes** — `execute_model()` now drops both table and view before creating either, handling view↔table transitions automatically.
- **Datagen geometric min** — `GeneratorSpec::Geometric` accepts optional `min: i32` to prevent zero values.

See [plan](plans/20260409-smelt-shop-fixes.md) for full details.

### ~~Packaging — Source Distribution & Python 3.14 Wheels~~ ✅ (April 10, 2026)

- Added `build-sdist` job to release workflow using `maturin sdist`
- sdist included in PyPI and TestPyPI publish steps
- `bindings = "bin"` in pyproject.toml produces `py3-none-{platform}` wheels, compatible with Python 3.9–3.14 on all platforms

### ~~Testing Strategy Improvements~~ ✅ (April 10, 2026)

- Added `examples/ecommerce/` workspace (19 models, 2 seeds, 3 sources) as regression scaffold
- Added `ecommerce_no_diagnostics` test to `example_diagnostics.rs`
- Added `ecommerce_execution.rs` compile-and-execute integration test against DuckDB
- Property tests cover CTEs, set operations, joins, and type inference across full model patterns

### ~~LSP Refactorings & Code Actions~~ ✅ (April 5-6, 2026)

Full refactoring support in the LSP: rename (CTEs, models, sources, columns with cross-file lineage tracing), code actions (CAST fixes, create model, add source/column, extract CTE, inline CTE), and find-references. All implemented as pure functions in smelt-db with thin LSP wrappers. Also fixed arrow 57→58 version mismatch and extracted duplicated functions to shared crates.

See [plan](plans/20260405-lsp-refactorings.md) for details.

### ~~LSP Goto-Definition & Column Diagnostics~~ ✅ (April 3-4, 2026)

Major LSP expansion: goto-definition now covers sources, CTEs, columns, and qualified references. Undeclared column reference diagnostics added. Python model LSP integration with real `ProjectContext`. Multiple stability fixes.

See [LSP & Editor Support](#lsp--editor-support) below for full details.

### ~~Code Quality & Hardening~~ ✅ (March 28, 2026)

All four sub-items completed:
- ✅ Snapshot tests: 30 `insta` tests for `smelt-dialect` covering all dialect rewrite paths
- ✅ CLI decomposition: `main.rs` split from 2,656 → 339 lines + 12 per-subcommand modules
- ✅ Structured logging: `tracing` crate replaces ~90 `println!`/`eprintln!` calls across 14 files
- ✅ unwrap() audit: ~35 production `unwrap()` → `expect("reason")` across 13 files

See [Code Quality & Hardening](#code-quality--hardening) below for details.

### ~~Data Testing Framework — `smelt test`~~ ✅ (March 27, 2026)

Fully implemented. See [Data Testing Framework](#data-testing-framework) below for details.

### ~~Data Catalog — `smelt docs generate`~~ ✅ (March 29, 2026)

Static data catalog / data dictionary generation. Outputs Markdown (default) or JSON.

- ✅ Per-model pages: description, owner, tags, materialization, columns with inferred types and lineage, upstream/downstream deps, incremental config
- ✅ Column enrichment: merges Salsa type inference with frontmatter descriptions and column-level tests
- ✅ Project index: model table, tag index, execution order
- ✅ JSON format: structured `catalog.json` for machine consumption
- ✅ `--select` filtering reuses existing selector infrastructure
- ✅ Nested subcommand (`smelt docs generate`) for future `smelt docs serve`

See [plan](plans/20260329-docs-generate.md) for details.

### ~~Schema Diff — `smelt diff`~~ ✅ (March 29, 2026)

Offline schema change detection. Compares inferred model schemas (from SQL parsing/type inference) against deployed schemas (`.smelt/schemas/`) without requiring a database connection.

- ✅ Per-model diff: column additions, removals, type changes, nullability changes
- ✅ Risk assessment: safe ALTER TABLE vs full refresh vs column removal flag
- ✅ `--select`/`--exclude` filtering reuses existing selector infrastructure
- ✅ `--json` output for CI integration (machine-readable)
- ✅ Exit code 1 when changes detected (CI-friendly)
- ✅ Removed model detection (deployed schema exists but model deleted from code)
- ✅ Per-model target resolution (works with multi-backend projects)

### ~~Schema Evolution~~ ✅ (March 30, 2026)

Efficient schema migrations using ALTER TABLE + DEFAULT values instead of full table refresh.

- ✅ Column `default:` in frontmatter — NOT NULL column additions use `ALTER TABLE ADD COLUMN ... DEFAULT val` instead of full refresh
- ✅ Column `backfill:` in frontmatter — SQL expression for UPDATE backfill after ALTER TABLE ADD COLUMN
- ✅ `schema_evolution: { strategy: full_refresh }` — opt out of ALTER-based migration per model
- ✅ Nullable-to-NOT-NULL with default — `UPDATE ... WHERE IS NULL` + `ALTER SET NOT NULL`
- ✅ `smelt diff` shows migration plan with defaults (ALTER with DEFAULT instead of full refresh)

### ~~Schema Evolution — Complex Types~~ ✅ (April 5, 2026)

Production schema evolution for nested/complex types (Struct, Array, Map). Previously, any change to a complex type column triggered a full table refresh.

- ✅ `parse_type()` extended for `STRUCT(...)`, `TYPE[]`, `MAP(K, V)` with recursive nesting
- ✅ `Map(Box<DataType>, Box<DataType>)` variant added to `DataType`
- ✅ Recursive type normalization (`DataType::normalize()`)
- ✅ Structural diff for complex types — field-level additions, removals, type widening, nested changes
- ✅ Safe widening rules for nested types (e.g., `INTEGER` → `BIGINT` inside a struct)
- ✅ Abstract `SchemaOperation` enum for backend-agnostic migration planning
- ✅ DuckDB DDL generation: struct dot-notation, `struct_pack` rewrites, `list_transform` for array-of-struct
- ✅ Spark DDL generation: `mergeSchema` for safe additions, `TableRewrite` for unsupported operations
- ✅ Table format config (`format: delta|parquet`) at target and model level
- ✅ `--allow-full-refresh` CLI gate for expensive operations
- ✅ `default:` changed from YAML value to SQL expression string (breaking change)
- ✅ Identifier quoting for SQL keywords and special characters
- ✅ Graceful fallback for unparseable type strings with warnings
- ✅ Round-trip verification: `DataType` → `to_sql()` → `parse_type()` → `DataType`
- ✅ User-facing documentation on smeltsql.com (schema-evolution guide, backend capability matrix)

See [plan](plans/20260405-schema-evolution-complex-types.md) for details.

### ~~Spark / Databricks Backend~~ ✅ (March 28, 2026)

Spark backend implemented via PySpark/PyO3 bridge. All Backend trait methods are now functional, connecting to Spark through PySpark's SparkSession.

- ✅ PySpark bridge via PyO3 — thin Python adapter (`spark_adapter.py`) wraps SparkSession
- ✅ SQL execution with zero-copy Arrow result conversion (`pyarrow.Table` → `RecordBatch` via C Data Interface)
- ✅ Table/view materialization (DROP + CREATE TABLE AS, CREATE OR REPLACE VIEW)
- ✅ Incremental support: DELETE+INSERT, MERGE INTO, INSERT OVERWRITE, APPEND
- ✅ Catalog/schema management (three-part names: `catalog.schema.table`)
- ✅ pyo3 upgraded from 0.24 → 0.26 (required for arrow-pyarrow compatibility)
- ✅ Works with local Spark Connect, Databricks Connect, EMR, Dataproc
- 🔮 Integration test parity with DuckDB tests (requires Spark Connect server)
- 🔮 Authentication configuration docs (tokens, OAuth, instance profiles)

---

## Code Quality & Hardening ✅ (March 28, 2026)

### Structured Logging ✅

- `tracing` crate with `EnvFilter` (controlled via `RUST_LOG` env var)
- ~90 `println!`/`eprintln!` calls converted to `tracing::info!`/`debug!`/`warn!` across 14 files
- Program output (tables, JSON, test results) kept as `println!` for piping

### Error Handling ✅

- ~35 production `unwrap()` calls replaced with `expect("reason")` across 13 files
- Focused on smelt-cli, smelt-db, smelt-core, smelt-backend-duckdb
- Test code left as-is (idiomatic Rust)
- Remaining `unwrap()` calls are in test code or already have proper error handling

### CLI Decomposition ✅

- `main.rs` split from 2,656 → 339 lines (arg structs + dispatch only)
- 11 per-subcommand modules under `src/commands/` (run, backbuild, seed, build, status, history, explain, table, type, ui, test)
- Shared utilities extracted to `src/helpers.rs` (352 lines)

### Snapshot Testing ✅

- 30 `insta` snapshot tests for `smelt-dialect` printer
- Covers all dialect rewrite paths: QUALIFY, ARRAY, DATE, `::` cast, trailing comma, function remapping, ref/source resolution, ephemeral refs, combined rewrites
- All three dialects tested: DuckDB, SparkSQL, PostgreSQL

---

## Data Testing Framework ✅ (March 27, 2026)

### Test Types
- **CTE isolation tests**: Test a single CTE by mocking all its direct dependencies
- **Whole-model tests**: Test entire model by mocking `smelt.ref()` inputs
- **Singular tests**: Custom SQL assertion tests (`materialization: test`, pass when 0 rows returned)
- **Property-based tests**: Omit columns from inputs → framework generates random values using type inference, runs N times (configurable via `test.cases`)
- **Column-level data quality tests**: `not_null`, `unique`, `accepted_values`, `min`, `max` defined in model frontmatter

### CLI
- `smelt test` with `--select`, `--verbose`, `--show-all`, `--seed` flags
- Tests excluded from `smelt run`/`build`/`explain`
- Example tests across ephemeral_demo, retail_analytics, timeseries projects

### Remaining work
- `smelt docs generate` for data catalog / data dictionary output
- Recursive CTE support in test isolation
- Snapshot/golden file mode (auto-capture expected output)
- LSP validation of test references (`test.model`, `test.target_cte`)
- Seed data integration with tests
- Statically-checkable assertions and type-system-leveraged testing (exploratory)

---

## Language & Parser

**Current state**: Full SQL parser with error recovery (Rowan CST), covering SELECT, FROM, JOIN (all types), WHERE, GROUP BY, HAVING, ORDER BY, LIMIT, CTEs, window functions, set operations, subqueries, QUALIFY, lambda expressions, array/struct/JSON literals, and all standard operators.

- smelt extensions: `smelt.ref()`, `smelt.metric()`, `smelt.source()` with `=>` named parameters
- Trailing commas in SELECT/GROUP BY
- YAML frontmatter for model configuration
- Python model support via `@model` decorator (subprocess + optional PyO3)
- Multi-dialect superset: PostgreSQL base with DuckDB and Spark features
- PIVOT/UNPIVOT: rejected with diagnostic error (not yet supported, March 31, 2026)
- Parser structural assertion tests and AST accessor bug fixes (April 3, 2026)
- Fixed bare-token problem and implicit alias detection (April 3, 2026)

**Next steps**:
- ~~Smelt Functions Steps 1–5~~ ✅ (April 22–24, 2026) — `smelt.define`, `smelt.fn.*`, `TableExpr`, call-site type checking, LSP hover, context binding, CTE-derived `SelectItems` contexts, `session_rollup` end-to-end, Tier 2/3 body/return checking, Tier 2 → Tier 1 inline expansion, bidirectional generics. See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- ~~Smelt Functions Step 6 (PASSING clauses)~~ ✅ (April 24, 2026) — Phase 28 (parser: context-sensitive `PASSING name AS (...)` syntax) and Phase 29 (binding + type-checking) complete. `session_rollup` demonstrated with block-syntax `PASSING metrics AS (COUNT(*))` in `examples/functions_demo/`. `UnknownPassingParameter` diagnostic, LSP code mapping, and basic `body_expr()` / `name_range()` AST helpers added. LSP cursor-in-body column completion deferred (see Phase 29 deferral note in plan). Steps 7–8 (planner, struct row vars) remain. See [plan](plans/20260422-smelt-functions.md).
- ~~Smelt Functions Step 7 Phases 30–34~~ ✅ (April 25, 2026) — Phase 30: `smelt-planner::logical::LogicalNode::FunctionCall` with `transparent` flag and `FunctionProperties`; `logical_plan` Salsa query in `smelt-db`. Phase 31: column provenance declared via per-declaration frontmatter `provenance:` key, gated by `unstable_schema: true` in `smelt.yml` (`DiagnosticCode::UnstableSchemaRequired` fires when the flag is absent). Phase 32: `PlannerRule` trait, `RuleResult`, `RuleContext`, `apply_rules_to_fixed_point` fixed-point loop, and `ExpandTransparentFunctionCalls` rule. Phase 33: `PushFilterIntoTransparentFunction` rule. Phase 34: `EliminateUnusedLeftJoin` rule — elides a `LogicalNode::LeftJoin` whose RHS columns are unused in the parent projection list, when cardinality is declared `1:1`. Demo: `enriched_order` function (declares `joins:` with 1:1 cardinality against `dim_customer`) + `order_totals` model (projects no dimension columns → join eliminated). Soundness caveat documented in §20E: the rule trusts the declared cardinality without data verification. Step 7 complete.
- ~~Smelt Functions Step 8 Phases 35–38~~ ✅ (April 25, 2026) — Phase 35: `STRUCT_TYPE`, `ROW_TAIL`, `BRACE_STRUCT_LITERAL`, `SPREAD_ITEM` syntax kinds; `SmeltType::Struct { fields, tail }` + `StructRowTail` in smelt-types; two-named-row-var constraint check. Phase 36: call-site struct row-var unification (`check_struct_row_var_binding`), extras bound via `set_row_var_binding`, spread-item erasure for `..event` in bodies. Phase 37: return-type row-var resolution — `Expr<Struct<{hour: BigInt, ..r}>>` return resolves to a concrete `DataType::Struct` at call sites; `BraceStructLiteral` type inference in `infer_brace_struct_literal_type`; LSP hover shows expanded fields. Phase 38: `smelt.as_struct(<alias> [EXCEPT <cols>])` expression — `SMELT_AS_STRUCT_CALL` syntax node, `SmeltAsStructCall` AST wrapper, `infer_as_struct_type` resolving columns via `TypeContext::columns_for_qualifier`, `as_struct_to_sql` emitting DuckDB/Spark/Postgres backend SQL, `AsStructUnsupportedBackend` diagnostic for functions declaring unsupported backends. Step 8 and the full smelt-functions v1 experimentation roadmap are complete.
- ~~Smelt Functions Steps 9–13 (plan review + polish) Phases 39–53~~ ✅ (April 26, 2026) — 14 phases closing the 28 review findings from the plan's §20 audit. Key deliverables: Phase 39 (`--show-plan` CLI flag wiring logical-plan rule pipeline end-to-end), Phase 40 (CAST emission resolved from canonical-return registry), Phase 41 (transparent-call body splice into logical plan), Phase 42 (list-splice comma elision at lowering), Phase 43 (`as_struct` backend SQL emission), Phase 44 (canonical fixture tightening: `safe_divide` body guards, `monitored_session_rollup`), Phase 44b (fragment-forward parser + `SelectItems<K, ctx>` type system), Phase 45 (JOIN alias visibility in `TableExpr`-returning bodies), Phase 46 (`TableExpr` argument shapes: CTEs, derived tables, subqueries), Phase 47 (cross-function CTE schema inference, drop opaque-CTE suppression), Phase 48 (LSP hover wiring, `PASSING` completion, multi-level frame trace), Phase 49 (`WindowInScalarContext` deep-walk into scalar subqueries), Phase 50 (built-in registry expansion: arithmetic operators, missing aggregates, window functions), Phase 51 (`provenance:` / `joins:` validator with `ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable` diagnostics), Phase 52 (missing-provenance pushdown advisory `Hint` + extern fragment-param rejection `ExternFragmentParamUnsupported`), Phase 53 (plan audit: empty commit-SHA cells filled, stale `Context` comment corrected, cross-file extern same-name collision fixture). See [plan](plans/20260422-smelt-functions.md).
- ~~Smelt Functions Phase 55 — `smelt.as_struct()` and `smelt.fn.*` SQL emission during `smelt build`~~ ✅ (April 27, 2026) — Wired both `smelt.as_struct()` and `smelt.fn.*` into actual SQL emission in the dialect printer. Added `AsStructEmitter` and `SmeltFnExpander` closure type aliases as optional fields on `PrintContext`; the printer's `SMELT_AS_STRUCT_CALL` and `SMELT_FN_CALL` handlers invoke them when present. In `SqlCompiler::compile()`: builds a `TypeContext` from the original SQL before ref-resolution so alias→columns mappings are available, constructs both closures from upstream schemas and function body maps, and passes them into `PrintContext`. Added `set_function_bodies()` on `SqlCompiler` for tests. `substitute_params()` does whole-word parameter substitution skipping string literals. 5 new tests in `crates/smelt-cli/tests/as_struct_emission_tests.rs` cover DuckDB struct literal emission, EXCEPT exclusion, function body expansion, pass-through when no body map, and TypeContext alias building. All tests pass, zero clippy warnings.
- Metrics DSL (Layer 1 — declarative metric definitions, `smelt.metric()` resolution)
- `smelt.param()` for parameterized models
- PIVOT/UNPIVOT support (currently rejected with diagnostic)

## Type System

**Current state**: Full type inference for expressions, functions, aggregates, window functions, and cross-model schemas. NULL tracking, row polymorphism (`SELECT *` propagation), and `resolved_model_schema()` Salsa query.

- Property-based testing against DuckDB and Spark (via `smelt-parser-compat`)
- Comprehensive generator coverage (March 29, 2026): 12 expression kinds (IS NULL, comparisons, unary NOT/minus, EXISTS, LIKE/ILIKE, regex, scalar subqueries, mixed-type binary ops, `::` cast), 5 query shapes (Scalar, GroupBy, GroupByHaving, GroupByWindow, Distinct), 10 base types (incl. Time, Interval), window frame specs
- LIKE/ILIKE parser support with type inference
- Known divergence registry for backend-specific type differences
- JSON operator type inference

**Next steps**:
- ~~LSP quick-fixes for type errors (CAST suggestions)~~ ✅ (April 5, 2026) — see [LSP Refactorings](#lsp-refactorings--code-actions--april-5-6-2026)
- LSP quick-fixes for COALESCE suggestions on NULLs
- Stricter boundary type checking (explicit input/output schemas)
- *See also*: snapshot tests for type inference output ([Code Quality & Hardening](#code-quality--hardening)), type-system-leveraged data testing ([Data Testing Framework](#data-testing-framework))

## Planner

**Current state**: `smelt-planner` crate with model-graph-level planning:

- Cube split: splits multi-`COUNT(DISTINCT)` queries into parallel sub-queries
- Incremental materialization: detects time-partitioned GROUP BY, generates DELETE+INSERT
- Temporal dependency inference: analyzes window functions, LAG/LEAD, JOIN intervals to determine lookback/lookahead requirements
- Batch safety analysis: classifies models as FullyBatchSafe/BoundedSafe/PerPartitionOnly
- DAG-aware range computation for backfill planning

**Deferred**:
- ⏸️ Per-ref upstream filtering — wrapping `smelt.ref()` in filtered subqueries requires column lineage tracing through query AST; currently applies single wider filter range
- ⏸️ Custom time granularities — plugin API for fiscal quarters, 4-4-5 retail calendars; placeholder `Custom` variant exists
- ⏸️ Rule conflict resolution — how planner rules compose when they conflict (e.g., shared sub-expression vs incremental on same model); currently last-transformation-wins

**Next steps**:
- Three-level rule architecture: (1) Logical→Logical transforms with functions as opaque typed nodes, (2) Logical→Physical with strategy-dependent function expansion, (3) Physical→Execution plan with multi-statement orchestration. See [smelt functions discussion paper](research/20260413-smelt-functions.md) §8.
- Function-aware optimizations: join elimination for unused 1:1 LEFT JOINs, predicate pushdown into function blocks, cross-function fusion
- Shared materialization detection (multiple models computing same intermediate)
- Model fusion (trivial passthrough models)
- Cost-based optimization (requires backend statistics)
- Orchestrator integration — Dagster/Airflow plugin API (deferred to separate plan)

## Backends

**Current state**:
- **DuckDB**: Full implementation — table/view materialization, incremental DELETE+INSERT, bundled (no system install needed)
- **Spark**: Full implementation via PySpark/PyO3 bridge (March 28, 2026) — all Backend trait methods implemented, zero-copy Arrow conversion, works with Spark Connect and Databricks Connect. Requires PySpark in Python environment.
- **PostgreSQL**: Not started. Deprioritized in favor of Spark/Databricks.
- **Dialect printer**: `smelt-dialect` crate — single-pass CST walk emitting target SQL, handles QUALIFY, array literals, DATE literals, JSON function remapping

**Deferred**:
- ⏸️ Spark JSON incompatibilities — `TO_JSON(scalar)`, `JSON_CONTAINS`/`@>`/`<@`, `JSON_OBJECT`/`JSON_ARRAY` rewrites; compile-time warnings planned but not yet implemented

**Next steps**:
- ~~Spark/Databricks backend implementation~~ ✅ (March 28, 2026) — see [What's Next #1](#1-spark--databricks-backend)
- ~~Multi-backend execution in a single run~~ ✅ (March 25, 2026) — `BackendRegistry` with per-model `target:` frontmatter override, cross-backend validation
- ~~Cross-engine data exchange~~ ✅ (March 29, 2026) — cross-engine ref resolution via direct Parquet reads (no copy step); DuckDB resolves `smelt.ref('spark_model')` to `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet')`. Example at `examples/multi_engine/`. See [plan](plans/20260328-multi-engine-example.md).
- Integration test parity: run DuckDB integration tests against local Spark Connect
- *Deferred*: PostgreSQL backend

## LSP & Editor Support

**Current state**: Full LSP server (`smelt-lsp`) with Salsa incremental compilation:

- Diagnostics: parse errors, undefined refs, type errors, undeclared column references (with accurate positions)
- Go-to-definition for `smelt.ref()`, `smelt.source()`, CTEs, columns, and qualified references (e.g., `t.column`)
- CTE wildcard tracing (`SELECT *` through CTE chains)
- Hover with type information and model schemas
- Completions: model names, column names, table alias columns
- Python model awareness: real `ProjectContext` passed to Python models in LSP, valid ref targets, execution error diagnostics
- `sources.yml` live reload (changes update LSP without restart)
- Salsa 0.26 with `#[salsa::tracked]` free functions and `cycle_initial` fixpoint iteration (upgraded from 0.16)
- Find references for models, sources, and CTEs
- Rename: CTEs (single-file), models (cross-file with file rename), sources (cross-file + YAML), columns (full lineage tracing)
- Code actions: CAST quick-fixes, create model, add source/column to YAML, extract CTE, inline CTE
- VSCode extension with syntax highlighting and auto-activation
- CI verification: example workspaces checked for zero LSP diagnostics

**Recent** (April 3-4, 2026):
- ✅ Expanded goto-definition to sources, CTEs, columns, and qualified references
- ✅ CTE wildcard tracing for `SELECT *` column resolution
- ✅ Diagnostics for undeclared column references
- ✅ Python model LSP integration: real `ProjectContext` enables cross-boundary type inference
- ✅ Fixed LSP crash from Salsa cycle detection during memo validation
- ✅ Upgraded Salsa 0.16 → 0.26: `#[salsa::tracked]` free functions, `#[salsa::input]` structs, `#[salsa::accumulator]` diagnostics, `cycle_initial` fixpoint iteration; removed `catch_unwind` workaround (April 18, 2026)
- ✅ Fixed `sources.yml` changes not updating LSP until reload
- ✅ Fixed 35 LSP diagnostics across example workspaces + CI verification gate
- ✅ Fixed Python model `E2BIG` error on large projects and PyO3 `dict_items` extraction

**Next steps**:
- Dialect-specific informational hints ("QUALIFY will be rewritten for PostgreSQL")
- Optimizer opportunity suggestions as code actions
- Code action: extract to model (promote subquery/CTE to a new smelt model)

## CLI & Execution

**Current state**: `smelt-cli` with full pipeline:

- `smelt run` — execute models with optional `--start`/`--end` for incremental ranges, `--dry-run`, `--full-refresh`, `--auto` (range from interval store)
- `smelt backbuild` — target-focused rebuild with DAG-aware range expansion
- `smelt explain` — dependency graph + JSON export
- `smelt status` — interval coverage and gaps for incremental models
- `smelt history` — run history with model filtering
- `smelt test` — data testing framework (CTE isolation, whole-model, singular, property-based, column-level tests)
- `smelt type` — function type signatures
- `smelt docs generate` — static data catalog (Markdown/JSON) with column types, lineage, descriptions, tests (March 29, 2026)
- `smelt diff` — offline schema change detection, compares inferred vs deployed schemas without database connection (March 29, 2026)
- Smart batching based on batch safety analysis
- `smelt-state` crate for run manifests + interval tracking (`.smelt/` directory)
- Two-stage graph architecture: `LogicalGraph` (user intent) → `PhysicalGraph` (execution plan)
  - `LogicalGraph` with eagerly-resolved config per node (March 26, 2026)
  - `PhysicalGraph` with strategy resolution, ephemeral resolver ownership (March 26, 2026)
  - Graph-level planner transformations: `CreateNode`, `RemoveNode`, `RedirectRef`, `SetMaterialization` (March 26, 2026)
  - `smelt explain` shows physical execution plan with strategies, ephemerals, planner optimizations (March 26, 2026)

**Next steps**:
- ~~`smelt test`~~ ✅ (March 27, 2026) — see [Data Testing Framework](#data-testing-framework)
- ~~`smelt docs generate`~~ ✅ (March 29, 2026) — see [What's Next](#1-data-catalog--smelt-docs-generate)
- ~~`smelt diff`~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-diff--smelt-diff)
- `smelt check` — LLM-optimised diagnostic CLI ([design doc](plans/20260405-smelt-check.md))
- ~~Schema evolution with efficient migrations~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-evolution)

## UI Dashboard ✅ Phases 1-4 (March 24-25, 2026)

**Current state**: Web dashboard (`smelt-ui`) with React frontend and Axum backend:

- Phase 1: Live backend with file watching and WebSocket updates
- Phase 2: Full REST API, batch safety diagnostics, type information in UI
- Phase 3: Run planner with interactive preview, select/exclude with CLI command preview
- Phase 4: Run execution and monitoring with real-time WebSocket progress streaming
- Model graph visualization with dependency explorer
- Run history with expandable model details
- Model sidebar with type signatures and metadata

**Next steps**:
- See [docs/plans/20260324-ui-dashboard-expansion.md](plans/20260324-ui-dashboard-expansion.md) for Phases 5-6

## Ecosystem

**Recent** (March 25 – April 4, 2026):
- ✅ Documentation site for smeltsql.com (MkDocs Material, 15+ pages covering all features)
- ✅ Frontmatter validation with `deny_unknown_fields` (catches typos like `materialized:` vs `materialization:`)
- ✅ Multi-model file discovery with `ModelId` (`--- name: model_name ---` delimiters)
- ✅ Testing documentation: guide, CLI reference, and project structure docs
- ✅ ACE-FCA workflow: slash commands, tutorial, and artifact directories for structured development (March 31, 2026)
- ✅ SQL dialect analysis report: confirmed multi-dialect superset approach is sound (March 30-31, 2026)
- ✅ System DuckDB as default build mode — faster builds, no bundled C++ compilation (April 3, 2026)
- ✅ CI verification: example workspaces checked for zero LSP diagnostics (April 3, 2026)
- ✅ CI release builds fixed for bundled-duckdb feature (April 4, 2026)

- ✅ smelt-datagen bundled in `smelt-sql` PyPI wheel and standalone archives (April 9, 2026)
- ✅ smelt-datagen documentation: guide page on smeltsql.com covering all features (April 9, 2026)
- ✅ New datagen generators: `date`, `timestamp`, and `string_pattern` for realistic test data (April 9, 2026)

**Next steps**:
- Pre-built binaries via GitHub Releases (dev-release.yml workflow exists)
- Source distribution (sdist): verify the `maturin sdist` job actually ships on release — `release.yml` currently builds the four platform wheels but has no sdist job, despite the April 10 packaging work claiming one. (Wheels themselves are resolved: `bindings = "bin"` produces `py3-none` wheels covering Python 3.9–3.14 on all four platforms, so there is no remaining cp314-specific gap.)
- Datagen: geometric distribution `min` parameter (currently can produce 0, unsuitable for quantity fields)
- dbt-to-smelt cheat sheet showing common pattern equivalents
- Publish Python SDK to PyPI (currently TestPyPI only)
- Generic LSP configuration guides for Neovim, Emacs, and JetBrains
- Community readiness: `CODE_OF_CONDUCT`, issue/PR templates, and a "good first issue" onboarding path (repeatedly flagged across codebase reviews, no roadmap presence until now)

## Deferred-Work Backlog (untracked follow-ups)

Concrete work deferred during plan implementation (`docs/plans/`) that is not otherwise tracked above. Listed so it is *visible* and can be triaged into the queue — inclusion here is identification, not commitment. Maintained as part of [What's Next #3](#3-silent-failures--code-health-hardening). Grouped by area; each item cites its source plan.

**Type system / function registry**
- `to_seconds` returns `Interval` but inference doesn't recognise it — forces `epoch_us()` microsecond-arithmetic workarounds in models (`20260517-web-analytics-4-sessionize.md`).
- `md5` absent from the function registry — emits a Warning that fails the diagnostics gate; models fall back to `CONCAT` surrogate keys (`20260517-web-analytics-4-sessionize.md`).
- `arg_min` deliberately unregistered (only `arg_max` shipped), pending a real call site (`20260517-web-analytics-5-forward-only.md`).
- `ArgTypeMismatch` message format diverges from `functions.md` §8 ("expected X, got Y") (`20260422-smelt-functions.md`).
- `infer_expression_kind` parallel-walker gap — sub-expression kinds nested in array/struct/`ROW(...)`/`IN`/`EXISTS` silently drop to `Scalar` (overlaps the silent-`Unknown` front in #3) (`20260422-smelt-functions.md`).
- Per-arg-type canonical-return resolution — decimal precision is encoded per-signature today, not per-arg-type (`20260422-smelt-functions.md`).

**Functions / TableExpr**
- `f(x).field` single-field projection + `..r` row-tail struct-spread descoped — no field-postfix on a function call; schema/codegen disagree on row-tail spread (`20260527-function_schema_inference.md`).
- `build_fn_body_map` vs `build_fn_body_map_from_model_files` duplication (also called out in #3) (`20260519-functions-meta-1-call-expansion.md`).
- Phase 57 deferred function tests: FROM-position aliasing, Spark struct-literal lowering, literal-`VALUES` models (`20260519-functions-meta-gaps.md`).

**Meta-language**
- `smelt.sources.with_tag |> map(...)` generator driver yields zero emissions at runtime — only `load_yaml`/`load_json` drivers are enumerated; `staging_from_sources` ships a hardcoded `[ModelDef …]` workaround (`20260509-meta-language-E2.md`).
- `emitted_models` does an O(workspace) re-scan on any single-file edit (Salsa input granularity) (`20260509-meta-language-E2.md`).
- Unshipped `List<T>` derivations (`flat_map`, `zip_with`, `take`, `drop`, `any`, `all`, `find`, `partition`, …) + user-defined reducers; ship only if examples force them (`20260509-meta-language-overall.md`).

**VALUES / typing**
- `LATERAL (VALUES …)`, bare top-level `VALUES`, and `INSERT … VALUES` are untyped (`20260528-values-derived-table-typing.md`).

**Diagnostics encoding**
- `body_position_to_byte` counts codepoints, not UTF-8 bytes — non-ASCII emission bodies get shifted diagnostic positions (one-line fix pending a fixture) (`20260529-emission-body-diagnostics.md`).
- `smelt-ui` `DiagnosticInfo` still uses legacy `offset_to_position` — must move to `LineIndex` before that function is deleted (`20260530-byte-offset-diagnostics.md`).

**CLI ergonomics**
- No project-wide compile-only flag — extend `--show-plan` to the whole project vs a `smelt build --dry-run` is undecided (`20260502-smelt-loop-findings.md`).
- `smelt build --verbose` produces no extra output despite advertising compiled-SQL display (`20260502-smelt-loop-findings.md`).
- `smelt build --show-plan <generator>.sql` prints only the top-level loader node, not the emitted `ModelDef` paths (`20260509-meta-language-G.md`).
- `smelt test` silently skips `materialization: test` files with a boolean-`SELECT` body — no "discovered but skipped" diagnostic (`20260509-meta-language-G.md`).
- `smelt seed` command fate undecided (repurpose as `list`/`describe` vs remove; seed type-inference + caching open) (`20260406-seed-schema.md`).

**Datagen / incremental**
- Foreign-key resolution inside `JsonObject`/`entity.columns` always resolves to id 1 (`20260517-web-analytics-1-datagen-json-object.md`).
- Cumulative reprocessing detection has no watermark store — refuses via a heuristic pending a state-tracking spec (`20260523-cumulative-aggregate.md`).

**Cross-engine**
- Partition-level Parquet reads (only changed partitions vs full glob) + cross-engine schema validation / type coercion (Spark `STRING` vs DuckDB `VARCHAR`) (`20260328-multi-engine-example.md`).

**Documentation gaps** (from the `2026-05-30` bug ledger)
- `test` materialization mode missing from `materializations.md` (BUG-022).
- Calendar-invalid seed date (`2025-02-30`) infers DATE then hard-fails at load — undocumented interaction; note in `seeds.md` (BUG-031).
- ~70 diagnostic codes undocumented — needs a `diagnostics.md` (BUG-052).
- `incremental_models.md` Known Divergences omits the window-function safety check as a third non-expanding classify site (BUG-062).
- `smelt docs` follow-ons: HTML output, `smelt docs serve`, column-lineage visualization, `smelt docs diff` (`20260329-docs-generate.md`).

---

## Future / Exploration

Items here are interesting design problems without committed timelines.

- **External models in the graph**: Non-smelt models (e.g., PySpark jobs, legacy pipelines) as first-class DAG participants. User-annotated output schema and temporal behavior (partition column, granularity). Configurable execution: smelt-triggered (command/webhook) or externally-managed. Enables gradual migration and mixed-technology pipelines. Smelt's backbuild range computation would account for these models' declared temporal mappings. Declaration format needs design work.
- **Virtual environments / plan-apply workflow**: Compare schemas across dev/prod without materializing; require approval before execution. Interesting state management problem — smelt's logical/physical graph split could enable lightweight virtual environments.
- **OpenLineage / column-level lineage**: Export model and column-level lineage in OpenLineage format for catalog integration (DataHub, Amundsen, Atlan). Internal lineage tracking partially exists — interesting graph analysis problem.
- **Substrait integration**: Portable plan representation, DataFusion interop
- **Smelt Functions — next frontiers**: Steps 1–13 are ✅ complete (April 2026). Remaining open design problems: generics in `smelt.define` (user-polymorphic functions, §16 #14 deferred), variadics in `smelt.define` (§16 #15), parameterized models (`smelt.param()`), metrics DSL integration (`smelt.metric()`), and full function-body SQL lowering (replacing `LogicalNode::Raw` placeholders with structured plan nodes for end-to-end `smelt build` code generation from function bodies). See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- **Python as meta-layer input** (not whole models): Today Python authors entire models via the `@model` decorator. A lighter mode would let a Python script *feed the meta layer* — emit values (lists, maps, records) that the typed meta-language consumes to generate models — rather than producing model SQL itself. This keeps generation logic in smelt's typed meta-language while using Python only as a data/config source (e.g. pulling a cohort list or schema map from an external system). Interface and the typed boundary between Python output and meta-language input need design work; relationship to existing YAML/JSON/TOML loaders (`smelt.config.load_*`) should be worked out so they share one ingestion model.
- **Learning from history**: Use run statistics to suggest optimizations
