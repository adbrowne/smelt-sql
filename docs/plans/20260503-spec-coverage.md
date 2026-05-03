# Spec Coverage Plan

> **For agentic workers:** Use `/smelt:spec` for each spec phase. Each spec is a standalone `/smelt:spec` session — read source material first, then draft to `docs/specs/`.

**Goal:** Create normative specs for all user-visible smelt functionality that currently has only guide/reference docs, establishing `docs/specs/` as the full oracle for implementation correctness, user-doc sync, and refactor safety.

**Motivation:** The spec-first workflow proved valuable (functions, incremental models, type system). Without specs for the remaining ~12 feature areas, user docs drift from implementation, AI-assisted refactors lack an oracle for "intentional vs. incidental behavior," and `/smelt:validate` has nothing to check against.

---

## Current Spec Inventory

| Spec | Coverage | Status |
|------|----------|--------|
| `architecture.md` | System pipeline, crate structure, `smelt.<path>` addressing | stable |
| `types.md` | DataType vocabulary, type inference, nullability | experimental |
| `functions.md` | `smelt.define`, `smelt.extern`, PASSING, `as_struct` | experimental |
| `incremental_models.md` | Time-partitioned materialization, DELETE+INSERT, batch safety | experimental |
| `planner_integration.md` | Planner consuming frontmatter properties | — |
| `scoping.md` | Parameter scoping, no-overlap rule | — |
| `expansion.md` | Query expansion and rewrite rules | — |
| `gradual_typing.md` | Three-tier annotation checking | — |

**Gap:** The core model surface, project configuration, all four materialization modes (except incremental), seeds, sources, CLI behavior, model selection, testing, schema evolution, Python models, LSP features, data catalog, and datagen have no normative spec.

---

## Phases

### Phase 1 — Core Model Layer

Everything else references these. Write first.

#### 1a. `models.md`

**Covers:** SQL model files, YAML frontmatter schema, naming conventions, the four materialization modes (view/table/ephemeral/test), how `smelt.models.<name>` addressing is assigned, tag inheritance.

**Does not cover:** Incremental configuration (already in `incremental_models.md`).

**Source material:**
- `docs-site/docs/guide/sql-models.md`
- `docs-site/docs/guide/materializations.md`
- `docs-site/docs/reference/language.md`
- `crates/smelt-core/src/config.rs` — `ModelMetadata`, frontmatter keys
- `crates/smelt-core/src/lib.rs` — model discovery

**Key sections to nail:**
- Complete frontmatter key table (materialization, tags, backends, event_time_column, unique_key, etc.) with types and defaults
- How `smelt.models.<name>` is derived from file path (normative rule, not just example)
- Ephemeral inlining semantics — when does inline happen, what are the constraints
- Test materialization — how it differs from `testing.md` assertions (test *materialization* vs test *assertions*)

**Invariant to document:** Logical model SQL must be pure (no conditionals, no macros, no `is_incremental()`). The planner is the only place where execution strategy appears.

**Commit message:** `spec: models — SQL model files, frontmatter, materialization modes`

---

#### 1b. `project_config.md`

**Covers:** `smelt.yml` key surface, target configuration (DuckDB, Spark), default resolution order (project defaults → model frontmatter), multi-target declaration, unstable feature flags.

**Source material:**
- `docs-site/docs/reference/smelt-yml.md`
- `docs-site/docs/guide/targets.md`
- `crates/smelt-core/src/config.rs` — `ProjectConfig`, `TargetConfig`

**Key sections to nail:**
- Complete `smelt.yml` key table with types, defaults, and semantics
- Default resolution order: which frontmatter keys can be set at project level and how model-level overrides them
- Multi-target: how `smelt run --target <name>` selects a target; what target names are reserved
- What `unstable_features` enables and why it's gated

**Commit message:** `spec: project_config — smelt.yml and target configuration`

---

### Phase 2 — Data Inputs

Seeds and sources are referenced by models and tests; spec them before the testing spec can be complete.

#### 2a. `seeds.md`

**Covers:** CSV file placement, `smelt.sources.<seed_name>` addressing (seeds share the `sources` namespace), column type inference rules, null handling, reload behavior.

**Source material:**
- `docs-site/docs/guide/seeds.md`
- `crates/smelt-core/src/seeds.rs`

**Key sections to nail:**
- How column types are inferred from CSV headers vs data rows (exact inference rules, e.g. what makes a column `Integer` vs `Text`)
- Null handling: empty strings, `\N`, quoted empty, `NULL` literal
- Reload behavior: full reload each run vs. skip if unchanged
- Why seeds are in the `smelt.sources` namespace (design rationale: uniform addressing for consumers)

**Commit message:** `spec: seeds — CSV reference data, type inference, reload semantics`

---

#### 2b. `sources.md`

**Covers:** `sources.yml` format, column declaration, `smelt.sources.<schema>.<table>` addressing, schema validation against declared columns, multi-engine source definitions, what happens when live schema diverges from declaration.

**Source material:**
- `docs-site/docs/guide/sources.md`
- `docs-site/docs/reference/sources-yml.md`
- `crates/smelt-core/src/sources.rs`

**Key sections to nail:**
- Addressing: how `smelt.sources.X.Y` resolves to a specific `sources.yml` entry
- Schema declaration is advisory vs. enforced: which checks run at compile time vs. runtime
- Multi-engine: how a single source can have different connection info per target
- The distinction between sources (not managed by smelt) and seeds (managed by smelt)

**Commit message:** `spec: sources — external table declarations and sources.yml`

---

### Phase 3 — Execution Interface

#### 3a. `cli.md`

**Covers:** All CLI subcommands (run, build, seed, test, table, type, status, history, explain, diff, ui, docs, backbuild), their flags, argument shapes, output formats (text vs `--json`), exit codes, `explain --json` schema for orchestrator integration.

**Source material:**
- `docs-site/docs/reference/cli.md`
- `crates/smelt-cli/src/main.rs` and `crates/smelt-cli/src/`

**Key sections to nail:**
- `build` = `seed` + `run` — exact order and behavior on partial failure
- `run` vs `backbuild` — when to use each, how they differ for incremental models
- `explain --json` schema — authoritative shape for orchestrator consumers
- `status` gap detection logic — how gaps are defined for incremental models
- Exit codes — `0` = success, `1` = model failure, `2` = user/config error; standardize

**Note:** This spec will be large. Consider whether `cli.md` covers all commands or splits into `cli-run.md` / `cli-inspect.md`. Prefer one file unless it exceeds ~400 lines.

**Commit message:** `spec: cli — command surface, flags, exit codes, explain schema`

---

#### 3b. `model_selection.md`

**Covers:** Selector syntax (`name`, `tag:X`, `+name`, `name+`, `+tag:X+`), `--select`/`--exclude` flag behavior, graph traversal semantics (upstream `+`, downstream `+`), multiple selector precedence, exclusion taking priority over inclusion.

**Source material:**
- `docs-site/docs/guide/model-selection.md`
- `crates/smelt-core/src/selector.rs`

**Key sections to nail:**
- Exact BNF or EBNF for selector syntax
- Graph traversal: `+name` means "name and all upstream dependencies", `name+` means "name and all downstream dependents" — state the definition formally
- Multiple `--select` flags: union or intersection?
- `--exclude` + `--select` interaction: is exclusion applied after inclusion or independently?
- Tag resolution: tags are declared on models; what if a tag has no matching models?

**Commit message:** `spec: model_selection — selector syntax and graph traversal semantics`

---

### Phase 4 — Data Quality

#### 4a. `testing.md`

**Covers:** Test materialization (test definitions embedded in model frontmatter), CTE-level tests, whole-model tests, property-based tests, singular tests. Mock data injection, assertion evaluation, `check_order`, `cases`, test failure messages.

**Note:** Does NOT cover data generation for loading fixtures — that's `datagen.md`.

**Source material:**
- `docs-site/docs/guide/testing.md`
- `crates/smelt-planner/src/` (test execution path)
- `crates/smelt-db/tests/` (test fixture examples)

**Key sections to nail:**
- Complete `inputs:` and `expect:` YAML schema (types, null representation, row ordering)
- CTE isolation mechanism: exactly how mock data replaces upstream dependencies at the CTE level
- Property-based test semantics: what `cases: N` means, reproducibility guarantee, what data is generated
- Singular tests: query real data, assert rows match expected; when is this appropriate vs mock tests
- Test failure output format — what does a failing test display?

**Invariant to document:** Tests run entirely in-memory against DuckDB. No network, no external database required.

**Commit message:** `spec: testing — test types, mock injection, assertion semantics`

---

#### 4b. `datagen.md`

**Covers:** `smelt-datagen` YAML config format, entity pools, foreign key referential integrity, Parquet output, partitioning support, determinism guarantees.

**Source material:**
- `docs-site/docs/guide/datagen.md`
- `crates/smelt-datagen/src/`

**Key sections to nail:**
- YAML config schema (complete key table)
- Entity pool semantics: cardinality, sampling distribution
- Foreign key enforcement: how referential integrity is maintained during generation
- Determinism guarantee: same seed → same output, document the seed mechanism
- Parquet output: schema inference, partition directory layout

**Commit message:** `spec: datagen — deterministic test data generation`

---

### Phase 5 — Advanced Features

These are independent of each other; order within Phase 5 is flexible.

#### 5a. `schema_evolution.md`

**Covers:** Safe vs. unsafe schema changes (additive is safe; removal, type changes are unsafe by default), `--allow-column-removal`/`--allow-full-refresh` flags, `smelt diff` output format, struct/array/map field-level diffs, view↔table transitions.

**Source material:**
- `docs-site/docs/guide/schema-evolution.md`
- `crates/smelt-backend-duckdb/src/`
- `crates/smelt-backend-spark/src/`

**Key sections to nail:**
- Classification table: which changes are safe, unsafe-but-allowed-with-flag, always-blocked
- `diff` output format — authoritative schema for the JSON output
- Struct/array/map field-level change detection: how nested type changes are classified
- View↔table transition: what happens when `materialization:` changes from `view` to `table`

**Commit message:** `spec: schema_evolution — change classification and ALTER TABLE strategy`

---

#### 5b. `python_models.md`

**Covers:** Python model files, `@model` decorator, return type (`str` — SQL string), project metadata access API (`project.models_with_tag()` etc.), multiple models per file, compile-time evaluation.

**Source material:**
- `docs-site/docs/guide/python-models.md`
- `crates/smelt-planner/src/` (Python execution path)

**Key sections to nail:**
- When Python is evaluated: compile time only, no runtime Python
- What project metadata API is available inside `@model` functions (exact method signatures)
- Multiple `@model` functions per file: ordering, naming conventions
- Error handling: what happens when Python raises an exception during compilation
- Why compile-time Python rather than Jinja templates (design rationale)

**Commit message:** `spec: python_models — compile-time Python SQL generation`

---

#### 5c. `lsp.md`

**Covers:** Language Server Protocol feature set — diagnostics (undefined refs, type errors), go-to-definition (models, CTEs, columns, sources), find-all-references, hover (schema, lineage, types), rename refactoring, completions, code actions (create missing model, CAST fix, extract CTE).

**Source material:**
- `docs-site/docs/guide/editor-features.md`
- `docs-site/docs/guide/editor-setup.md`
- `crates/smelt-lsp/src/`

**Key sections to nail:**
- Feature matrix: which features work across which identifier types (model refs, CTE names, column names, source refs, function names)
- Diagnostic categories: what produces an error vs. warning vs. hint
- Rename scope: what is updated when a model is renamed (all refs in project, not just current file)
- Code action preconditions: when does "create missing model from ref" appear?
- Performance contract: Salsa-based incremental updates; changes propagate without full reparse

**Commit message:** `spec: lsp — LSP feature set, diagnostic categories, rename scope`

---

#### 5d. `data_catalog.md`

**Covers:** `smelt docs generate` output format (markdown and JSON), per-model catalog pages, column description sources, project index, `smelt docs list`/`smelt docs show` embedded CLI docs.

**Source material:**
- `crates/smelt-cli/src/docs.rs`

**Key sections to nail:**
- `smelt docs generate` output structure: directory layout, file naming, what each file contains
- JSON catalog schema: authoritative shape for programmatic consumers
- Column description sources: where do column descriptions come from (frontmatter comments, explicit annotations, inferred)?
- Embedded docs (`smelt docs show <topic>`): how topics are registered, what's available

**Commit message:** `spec: data_catalog — docs generate output, JSON schema, embedded CLI docs`

---

## Ordering Rationale

Foundation first follows the dependency graph among specs themselves:
- `models.md` and `project_config.md` are referenced by nearly every other spec
- `seeds.md` and `sources.md` define input data that `testing.md` mocks
- `cli.md` and `model_selection.md` need model and source concepts to be stable
- `testing.md` builds on seeds + sources + model concepts
- Phase 5 specs are independent of each other

## Spec Hygiene Note

After creating new specs, review the four "thin" existing specs (`planner_integration.md`, `scoping.md`, `expansion.md`, `gradual_typing.md`) for completeness. These may need Design and Known Divergences sections fleshed out now that the full spec surface is established. This is separate work — not part of this plan — but flag any gaps found during Phase 1–5 work.

## Verification Per Spec

For each spec, after drafting:
1. Open the corresponding `docs-site/` guide page and confirm every user-visible item is either in the spec's Surface or deliberately marked out-of-scope
2. Check `crates/smelt-core/src/` and the relevant backend crate for any implemented behavior not yet captured
3. Run `/smelt:validate <feature>` once that command supports the new spec

## Progress Tracking

| Spec | Status |
|------|--------|
| `models.md` | done (416e49b) |
| `project_config.md` | done (b33d92f) |
| `seeds.md` | done (a0cdd47) |
| `sources.md` | done (a0cdd47) |
| `cli.md` | done (a1b17f0) |
| `model_selection.md` | done (a1b17f0) |
| `testing.md` | done (b94b05f) |
| `datagen.md` | done (b94b05f) |
| `schema_evolution.md` | done (f55e5e0) |
| `python_models.md` | done (f55e5e0) |
| `lsp.md` | done (f55e5e0) |
| `data_catalog.md` | done (f55e5e0) |
