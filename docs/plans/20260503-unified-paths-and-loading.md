# Plan: Unified `paths:`, kind-by-content resolver, and backend-portable seed loading

**Date**: 2026-05-03
**Spec**: [`docs/specs/seeds.md`](../specs/seeds.md), [`docs/specs/sources.md`](../specs/sources.md), [`docs/specs/architecture.md`](../specs/architecture.md), [`docs/specs/smelt_yml.md`](../specs/smelt_yml.md), [`docs/specs/cli.md`](../specs/cli.md)
**Spec diff**: commits `2c13fd9` (seeds rewrite) and `51384ca` (cross-cutting split). Treat as one diff — `seeds.md` references `sources.md`, `architecture.md` §"Resolution", §"Default materialization name mapping", and §"Backend trait surface"; the implementation must move together.
**Tracking PR / branch**: branch `worktree-seeds` (PR # TBD)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the five specs above — they are the correctness oracle. Do not re-open settled spec decisions; in particular, the *hard-cut* migration policy (no `model_paths`/`seed_paths`/`sources.yml` compat shim) is settled.
2. Confirm you are on branch `worktree-seeds`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.
- An example workspace's expected DB-name output changes by more than the path-join rule predicts (Phase 2 churn is large; do not silently absorb unrelated drift).

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature against a workspace under `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope (e.g. Phase 4 does not touch sources, Phase 6 does not change the CSV inferencer).
- Honor architectural invariants from `CLAUDE.md` (in particular `smelt-db` purity; new analysis logic is pure functions wrapped by Salsa queries).

---

## Context

The implementation today still uses `model_paths` + `seed_paths`, an aggregate root `sources.yml`, kind-by-directory inference, `<schema>.<leaf>` DB-name mapping with subdirectory-becomes-schema, and DuckDB's `read_csv_auto` for seed loading. The five specs above now describe a single unified shape: one `paths:` scan list, kind determined by file format/content with a `(.csv, .yml)` sidecar tiebreaker, addresses that are workspace-relative paths under any scan root with global uniqueness, default DB location `<target_schema>.<path-joined-by-_>`, per-entity source `.yml` files, and a smelt-owned CSV parser feeding `Backend::load_table(...)`. This plan migrates the implementation in one branch with no compat shim.

## Scope

### In scope (spec coverage)

- `smelt_yml.md` Surface §"Top-level keys" — `paths:` replaces `model_paths`/`seed_paths`.
- `architecture.md` §"Resolution" — kind-by-content classification, `(.csv, .yml)` sidecar tiebreaker, cross-`paths:` address uniqueness.
- `architecture.md` §"Default materialization name mapping" — `<target_schema>.<path-joined-by-_>` for models, seeds, and sources.
- `architecture.md` §"Backend trait surface" — `load_table(schema, name, arrow_schema, batches)` on `Backend`, implemented for DuckDB and Spark.
- `seeds.md` Surface §"What a seed is", §"CSV format", §"Type inference", §"`smelt seed` lifecycle", §"LSP integration" — smelt-owned CSV parsing/inference, strict defaults, bounded type set, ephemeral materialization, missing-sidecar warning + "Pin schema" code action.
- `sources.md` Surface §"Filesystem layout", §"Source YAML shape", §"Discovery and addressing", §"LSP surface" — per-entity source `.yml`, shared YAML grammar, `name:` override, no aggregate `sources.yml`.
- `cli.md` References — text-only update of related-specs cross-reference.
- Bundled `examples/` migrated to the new shape.
- `docs-site/` pages reconciled to match the specs.

### Explicitly deferred

- `smelt migrate` tool. Migration of user projects is documented; tooling is a follow-up plan.
- "Re-pin schema from CSV" LSP code action (`seeds.md` §"LSP integration"). Spec'd as deferred; this plan ships only the "Pin schema" action and the missing-sidecar warning.
- Per-seed CSV override surface (delimiter / quote / header). Strict defaults only (`seeds.md` §"Design").
- Tests on seed/source columns. Awaits `tests.md` (`seeds.md` §"Known Divergences").
- `view` / `materialized_view` materialization for seeds. Hard error in v1.
- Configurable per-entity DB-name mapping (analogue of dbt `generate_schema_name`). The default-only rule is shipped here.
- Source-existence verification against the live database (`sources.md` §"Known Divergences").
- Drift-diagnostic between CSV and pinned YAML (column added/removed). Deferred to a future LSP plan.
- Ephemeral seed row-count threshold.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 0a1e91f | 2026-05-03 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |

---

### Phase 1: `smelt.yml::paths` unification

**Goal.** Replace `model_paths` + `seed_paths` with a single `paths:` list (`smelt_yml.md` Surface §"Top-level keys"). Hard cut: legacy keys are unrecognised (warning per `smelt_yml.md` §"Unknown keys"). All `examples/*/smelt.yml` migrate in this commit so discovery keeps working downstream.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs::tests::paths_defaults_to_models` — `Config::default()` produces `paths: vec!["models".into()]`.
- `crates/smelt-core/src/config.rs::tests::paths_round_trips` — `paths: ["models", "fixtures"]` deserialises and serialises back unchanged.
- `crates/smelt-core/src/config.rs::tests::legacy_path_keys_warn` — a `smelt.yml` with `model_paths:` or `seed_paths:` parses successfully (unknown-key warning rule) but the resulting `Config.paths` is the default. Assert via `parse_with_warnings(...)` that the warning text names the legacy keys.
- `crates/smelt-cli/tests/example_diagnostics.rs` — every example workspace under `examples/` produces zero LSP diagnostics with the new config (real-fixture coverage).

**Implementation shape.**
- `crates/smelt-core/src/config.rs`: drop `model_paths` and `seed_paths` fields and their `default_*` functions. Add `pub paths: Vec<String>` with `#[serde(default = "default_paths")]` and `fn default_paths() -> Vec<String> { vec!["models".into()] }`. Update every `Config { … }` literal in `tests` and consumers.
- Add a small warning emission for legacy keys: parse `smelt.yml` raw with `serde_yaml::Value`, then warn if `model_paths` / `seed_paths` keys are present at the top level. Emit through the existing config-warning channel (the same surface that handles unknown-key warnings).
- Migrate `examples/*/smelt.yml` to `paths: [...]` covering each project's actual model and seed directories (`examples/timeseries`, `examples/retail_analytics`, `examples/ecommerce`, `examples/multi_engine`, `examples/ephemeral_demo`, `examples/functions_demo`, `examples/test_workspace`, `examples/smelt_shop_min`, `examples/demo_workspace`, `examples/broken`, `examples/huge`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs` — field rename, defaults, tests.
- `crates/smelt-core/src/{discovery.rs,project.rs,graph.rs}` — minimal edits to consume `config.paths` as a single list (kept behaviour-preserving by walking the list and feeding both the model-discovery and seed-discovery code paths from it; the kind-by-content split lands in Phase 2).
- `crates/smelt-cli/src/{seed.rs,run.rs}` — call-site updates only.
- `examples/*/smelt.yml` — migrated.

**Docs touched.**
- `docs/specs/smelt_yml.md` — Known Divergences §"Implementation still has `model_paths` and `seed_paths`": mark resolved.
- `docs-site/docs/reference/smelt-yml.md` — replace `model_paths`/`seed_paths` rows with `paths:`.

**Review checklist** (material findings only):
- [ ] TDD tests above exist and assert what's specified.
- [ ] No code path still reads `model_paths` or `seed_paths`.
- [ ] Examples all parse with the new config.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` is green.
- [ ] User reference page mentions `paths:` exactly as the spec does.

**Commit.** `config: collapse model_paths/seed_paths into single paths key`

---

### Phase 2: Universal resolver — kind by content, sidecar tiebreaker, cross-path uniqueness, default DB-name mapping

**Goal.** Make discovery walk every directory in `config.paths` and classify each file by format/content per `architecture.md` §"Resolution", with the `(.csv, .yml)` sidecar tiebreaker. Establish address = workspace-relative path under the matching scan root. Detect cross-`paths:` address collisions as a hard workspace-load error. Switch the default DB-name mapping for models, seeds, and sources to `<target_schema>.<path-joined-by-_>` (`architecture.md` §"Default materialization name mapping").

**Pre-conditions.** Phase 1 done — discovery consumes a single `paths:` list.

**TDD tests to write first.**
- `crates/smelt-core/tests/resolver_kinds.rs::csv_resolves_to_seed` — a workspace with `paths: ["models"]` and `models/data/users.csv` produces a seed entity at `smelt.data.users`, no source entity.
- `crates/smelt-core/tests/resolver_kinds.rs::csv_with_sibling_yml_is_seed_with_sidecar` — `models/data/users.csv` + `models/data/users.yml` produces one seed with the YAML attached as a sidecar (no source entity at `smelt.data.users`).
- `crates/smelt-core/tests/resolver_kinds.rs::standalone_yml_is_source` — `models/external/api/orders.yml` (no sibling `.csv`) produces a source at `smelt.external.api.orders`.
- `crates/smelt-core/tests/resolver_kinds.rs::sql_bare_select_is_model` and `::sql_smelt_define_is_function` — content-based dispatch covers `.sql` files.
- `crates/smelt-core/tests/resolver_kinds.rs::cross_paths_collision_errors` — `paths: ["models", "fixtures"]` with `models/users.csv` and `fixtures/users.csv` is a workspace-load error naming both files.
- `crates/smelt-core/tests/resolver_kinds.rs::name_collision_within_dir_errors` — `data/users.csv` and `data/users.sql` (bare SELECT) is a workspace-load error.
- `crates/smelt-core/tests/default_db_name.rs::join_path_components_with_underscore` — `smelt.staging.orders` → `main.staging_orders`; `smelt.payments.seeds.lookup.regions` → `main.payments_seeds_lookup_regions`; `smelt.users` → `main.users`.
- `crates/smelt-core/tests/default_db_name.rs::ephemeral_and_define_have_no_db_name` — entities with `materialization: ephemeral` (model or seed) and `smelt.define` declarations do not produce a DB-name; asking for one is an error.
- `crates/smelt-cli/tests/example_diagnostics.rs` — every example still has zero diagnostics under the new resolver and DB-name rule.

**Implementation shape.**
- `crates/smelt-core/src/discovery.rs` (or a new `resolver.rs`): one walk over `config.paths`, producing `Vec<ProjectEntity>` whose variants are `Model | Function | Seed { sidecar: Option<PathBuf> } | Source | Test`. Address is the path stripped of the matching scan-root prefix, lower-cased segments joined with `.`.
- The classifier is a pure function `classify(path, file_content) -> EntityKind` (uphold `smelt-db` purity convention by extension — even though this lives in `smelt-core`, write it as a pure function).
- Cross-path uniqueness check: HashMap `address → (path, scan_root)`; on collision, return `WorkspaceLoadError::DuplicateAddress`.
- New helper `default_db_name(address: &SmeltPath, target_schema: &str) -> QualifiedName` returning `QualifiedName { schema: target_schema, table: address.segments.join("_") }`.
- Replace existing `SeedFile::qualified_name()` (`smelt-cli/src/seed.rs:24`) and any model-side equivalent with calls to `default_db_name`. Drop the subdirectory-becomes-schema branch in `discover_seeds`.
- Aggregate `sources.yml` keeps working in this phase (it is not removed until Phase 6); the resolver simply does not attribute sources to it. Walking `paths:` already discovers any per-entity `.yml` files; the standalone-YAML branch produces sources from those.

**Critical files.**
- `crates/smelt-core/src/{discovery.rs,project.rs,graph.rs,seeds.rs,sources.rs}`.
- `crates/smelt-cli/src/seed.rs` — drop subdirectory-becomes-schema; route through `default_db_name`.
- `crates/smelt-db/src/lib.rs` — update `resolve_ref` if it carries any kind-by-prefix branches.
- `examples/*/` — adjust any tests or expected outputs that depended on the old `<schema>.<leaf>` naming. Existing per-example baseline files (e.g. `expected_output.sql`) regenerate to the new `<schema>.<path-joined>` form.

**Docs touched.**
- `docs/specs/architecture.md` — Known Divergences: nothing to remove yet (resolver and name mapping are now matched by code).
- `docs-site/docs/concepts/project-structure.md` — describe kind-by-content + sidecar rule + cross-path uniqueness.

**Review checklist** (material findings only):
- [ ] Resolver tests above exercise every kind branch and the sidecar tiebreaker.
- [ ] Cross-path collision is a hard error (not a warning).
- [ ] No remaining call site uses subdirectory-becomes-schema for seed schemas.
- [ ] Default-DB-name helper is pure and used by every persisted-entity site.
- [ ] Example diagnostics test green; example baselines regenerated and committed in this phase.

**Commit.** `core: kind-by-content resolver and default db-name mapping (path-joined-by-_)`

---

### Phase 3: `Backend::load_table` trait method

**Goal.** Add `load_table(schema, name, arrow_schema, batches) -> Result<()>` to the `Backend` trait (`architecture.md` §"Backend trait surface"); implement for DuckDB (Appender) and Spark (`createDataFrame(...).saveAsTable(...)`). No callers yet — this phase sets the trait surface so Phase 4 can wire seed loading through it.

**Pre-conditions.** Phases 1–2 done.

**TDD tests to write first.**
- `crates/smelt-backend-duckdb/tests/load_table.rs::round_trips_arrow_batches` — build `RecordBatch`es with each spec'd type (BOOLEAN, INTEGER, DECIMAL(p,s), DOUBLE, DATE, TIMESTAMP, VARCHAR), call `load_table("main", "t", schema, batches)`, then `SELECT * FROM main.t` returns equivalent batches.
- `crates/smelt-backend-duckdb/tests/load_table.rs::nullable_columns` — NULL values in nullable columns survive the round trip; NULLs in `nullable: false` columns surface a backend error (Phase 5 will lift this into a smelt-level diagnostic).
- `crates/smelt-backend-spark/tests/load_table.rs::round_trips_via_create_dataframe` — same shape against the Spark backend (gated behind the Spark integration-test feature flag if one exists; otherwise opt-in via env-var as today).
- `crates/smelt-backend/tests/trait_object_safety.rs::backend_is_object_safe` — `&dyn Backend` compiles with `load_table` in the trait.

**Implementation shape.**
- `crates/smelt-backend/src/lib.rs`: add the method to `pub trait Backend`. Use `arrow::record_batch::RecordBatch` + `arrow::datatypes::SchemaRef` as the input types so both backends speak the same vocabulary.
- DuckDB: open an `Appender` (already used for some paths; consolidate). Map each Arrow column to the DuckDB Appender column type. Existing `Appender`-based code in `crates/smelt-cli/src/seed.rs` is the reference; this phase moves the mechanism behind the trait without yet rewiring callers.
- Spark: build a Java/Scala `Dataset<Row>` via the Spark Connect bindings already in use; call `saveAsTable("<schema>.<name>")`. If Spark Connect lacks a direct Arrow-batch path, write Parquet to the target's `warehouse:` and `CREATE TABLE … USING parquet LOCATION …` (consistent with the parquet-exchange convention recorded in user memory).
- No production caller wiring in this phase; `crates/smelt-cli/src/seed.rs` continues to use its current DuckDB ingest path until Phase 4.

**Critical files.**
- `crates/smelt-backend/src/lib.rs`.
- `crates/smelt-backend-duckdb/src/lib.rs`.
- `crates/smelt-backend-spark/src/lib.rs`.

**Docs touched.**
- `docs/specs/architecture.md` — Known Divergences: nothing to remove yet (callers still need to be wired in Phase 4).
- `docs-site/docs/developing/architecture.md` — note `load_table` as a trait method; one paragraph on the cross-backend Arrow ingest path.

**Review checklist** (material findings only):
- [ ] Trait method is `async`, takes `&self`, and is object-safe.
- [ ] DuckDB and Spark implementations cover every type in `seeds.md` §"Type inference".
- [ ] Round-trip test verifies values, types, and nullability.
- [ ] No caller wiring slipped in from Phase 4.

**Commit.** `backend: add load_table(schema, name, arrow_schema, batches) on Backend trait`

---

### Phase 4: Smelt-owned CSV parser + Arrow-batch inferencer

**Goal.** Replace DuckDB's `read_csv_auto` with a smelt-owned CSV parser and inferencer (`seeds.md` §"CSV format", §"Type inference", §"Design"). Compile-time inference uses 100 rows; runtime inference reads the whole file. Both phases share one code path producing Arrow `RecordBatch`es. Strict defaults only — no per-seed override surface.

**Pre-conditions.** Phases 1–3 done. `Backend::load_table` exists for both backends.

**TDD tests to write first.**
- `crates/smelt-core/tests/seed_inference.rs::detects_each_type` — one fixture CSV per type (BOOLEAN, INTEGER, DECIMAL(p,s) within `(18,4)`, DOUBLE, DATE, TIMESTAMP without TZ, VARCHAR) yields the expected `DataType`.
- `crates/smelt-core/tests/seed_inference.rs::precedence_order` — a column whose values match both INTEGER and DECIMAL is INTEGER; a column of `1.5e10` is DOUBLE not DECIMAL; a column of `2025-01-10T08:00:00Z` is VARCHAR (not TIMESTAMP — TZ suffix forces fallback per `seeds.md` §"Type inference").
- `crates/smelt-core/tests/seed_inference.rs::decimal_cap` — `(p, s) = (19, 0)` falls through to DOUBLE; `(p, s) = (10, 5)` (s > 4) falls through to DOUBLE; `(p, s) = (10, 4)` stays DECIMAL.
- `crates/smelt-core/tests/seed_inference.rs::compile_runtime_widening` — a CSV whose first 100 rows infer INTEGER but a row 200 contains `3.14` produces compile-time `INTEGER`, runtime `DECIMAL(...)`; the inferencer surfaces both views.
- `crates/smelt-core/tests/seed_inference.rs::all_empty_column_is_varchar` — a column that is empty in every row infers as VARCHAR.
- `crates/smelt-core/tests/seed_csv_parser.rs::strict_defaults` — comma-delimited, double-quoted, header-required parsing. UTF-8 BOM consumed silently; LF and CRLF line endings accepted (mixed too); tab-delimited file is a hard parse error pointing at the bad delimiter.
- `crates/smelt-core/tests/seed_csv_parser.rs::empty_cell_is_null` — empty cells produce NULL in every column type, including VARCHAR.
- `crates/smelt-cli/tests/seed_loading.rs::seeds_load_via_load_table` — a real CSV under `examples/timeseries/seeds/` loads into DuckDB via `Backend::load_table`. The DuckDB-only `read_csv_auto` path is gone.

**Implementation shape.**
- New module `crates/smelt-core/src/seeds/csv.rs` using the `csv` crate for tokenisation. Strict reader configuration: `b','`, `b'"'`, `has_headers(true)`.
- New module `crates/smelt-core/src/seeds/infer.rs` implementing the type-precedence rules (`BOOLEAN → DATE → TIMESTAMP → INTEGER → DECIMAL → DOUBLE → VARCHAR`) as a single pure function `infer_columns(rows: &[Record], sample_limit: Option<usize>) -> Vec<DataType>`. Two callers: compile-time (`sample_limit = Some(100)`) and runtime (`None`).
- Arrow-batch builder that takes inferred types and parsed rows, producing `RecordBatch`es with the right column types and NULL handling.
- Rewire `crates/smelt-cli/src/seed.rs` to: parse → infer (or read sidecar pin) → produce batches → call `backend.load_table(schema, name, arrow_schema, batches)`. Drop the existing DuckDB `read_csv_auto` ingest.

**Critical files.**
- `crates/smelt-core/src/seeds.rs` (split into a `seeds/` module if it grows).
- `crates/smelt-cli/src/seed.rs`.
- `crates/smelt-cli/tests/seed_loading.rs` (new or extended).

**Docs touched.**
- `docs/specs/seeds.md` — Known Divergences §"Implementation lags spec": delete the bullet item for the inferencer/`read_csv_auto`.
- `docs-site/docs/guide/seeds.md` — explain the bounded type set, strict CSV defaults, "what falls back to VARCHAR" examples (TZ suffixes, scientific-notation outside DECIMAL cap, etc.).

**Review checklist** (material findings only):
- [ ] One inferencer code path; compile and runtime call into it with different sample sizes.
- [ ] `read_csv_auto` references (in code, comments, and docs) are gone.
- [ ] Strict-defaults test rejects non-comma delimiters with a clear error.
- [ ] All seed loading goes through `Backend::load_table`.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` and the `seed_loading` real-fixture test are both green.

**Commit.** `seeds: smelt-owned csv parser and inferencer; load via Backend::load_table`

---

### Phase 5: Sidecar YAML semantics — pinning, hard errors, ephemeral seeds

**Goal.** Implement sidecar YAML behaviour for seeds (`seeds.md` Surface §"Sidecar YAML — seed-specific keys", Semantics §1–§3, §7). Column-set must match CSV header exactly; type-coercion failures are hard errors with file/row/column pointer; `nullable: false` violations are hard load-time errors; `materialization: ephemeral` desugars at compile time to a `VALUES (…)` CTE; `view`/`materialized_view` produce a hard error at load time.

**Pre-conditions.** Phases 1–4 done. CSV inferencer + `Backend::load_table` paths in place.

**TDD tests to write first.**
- `crates/smelt-core/tests/seed_yaml_validation.rs::column_set_must_match_header` — a sidecar declaring `columns: [a, b]` against a CSV with header `a,b,c` is a hard error naming the extra column. Reverse case (sidecar has extra) is also a hard error.
- `crates/smelt-core/tests/seed_yaml_validation.rs::type_coercion_failure_is_hard_error` — a sidecar pinning column `x: INTEGER` against a CSV value `not_a_number` aborts with `(file, row, column)` in the error.
- `crates/smelt-core/tests/seed_yaml_validation.rs::nullable_false_blocks_null` — a NULL row value in a `nullable: false` column is a hard error at load time.
- `crates/smelt-core/tests/seed_yaml_validation.rs::name_override_rejected_for_seed` — a sidecar with `name: foo.bar` is a hard parse error with a message referring users to source declarations.
- `crates/smelt-core/tests/seed_yaml_validation.rs::view_or_materialized_view_rejected` — `materialization: view` and `materialization: materialized_view` are hard errors at load time.
- `crates/smelt-core/tests/seed_ephemeral.rs::ephemeral_seed_produces_no_table` — a seed with `materialization: ephemeral` does not appear in `smelt seed`'s load list and no `CREATE TABLE` is issued.
- `crates/smelt-core/tests/seed_ephemeral.rs::ephemeral_seed_compiles_to_values_cte` — a model that references an ephemeral seed compiles to SQL containing a CTE with a `VALUES (…)` body, with explicit `CAST` per column. DuckDB executes the result and returns the seed's rows.
- `crates/smelt-cli/tests/example_diagnostics.rs` — example workspaces containing ephemeral seeds (add a small one to `examples/ephemeral_demo/` if not already present) have zero diagnostics.

**Implementation shape.**
- Extend `crates/smelt-core/src/seeds.rs` (or a sibling module) with a sidecar-validator: header-match check, per-row coercion against the pinned `DataType`, NULL-vs-`nullable` check. Errors carry `(path, row_index, column_name)`.
- Extend the materialization handling in `crates/smelt-cli/src/seed.rs`: skip `ephemeral`; reject `view`/`materialized_view` with a hard error; `table` (default) goes through `Backend::load_table` as in Phase 4.
- Compile-time ephemeral expansion: a new pass in the SQL generator that, when a `smelt.<path>` reference resolves to an ephemeral seed, splices a CTE `<path-joined>` AS (VALUES (…))` into the model and rewrites the reference. Each column is wrapped in a `CAST(... AS <type>)` so the CTE schema matches the seed schema. The dialect printer handles per-dialect `VALUES` quirks (already a printer responsibility per `architecture.md` Identity properties).

**Critical files.**
- `crates/smelt-core/src/seeds.rs` — sidecar validation.
- `crates/smelt-cli/src/seed.rs` — materialization dispatch.
- `crates/smelt-db/src/{type_inference.rs,schema.rs}` — ephemeral seed expansion in the model lowering path (pure function, called by a Salsa wrapper).
- `crates/smelt-dialect/src/printer.rs` — emit VALUES literal for the seed in DuckDB; verify Spark equivalence path.

**Docs touched.**
- `docs/specs/seeds.md` — Known Divergences §"Implementation lags spec": delete bullets for sidecar pinning, ephemeral, and `name:` rejection.
- `docs-site/docs/guide/seeds.md` — sidecar examples for pinning, ephemeral, and the `nullable: false` contract.

**Review checklist** (material findings only):
- [ ] Every coercion-failure error includes file path, row index, and column name.
- [ ] Ephemeral seeds never trigger `Backend::load_table`.
- [ ] Compile-time `VALUES`-CTE rewrite has a real-fixture test that DuckDB executes.
- [ ] `smelt-db` analysis for ephemeral expansion is implemented as a pure function called by a Salsa query (purity invariant from `CLAUDE.md`).
- [ ] No silent NULL substitution anywhere — every coercion failure aborts.

**Commit.** `seeds: sidecar pinning, hard errors on coerce/nullable, ephemeral materialization`

---

### Phase 6: Per-entity source YAMLs; remove aggregate `sources.yml`

**Goal.** Migrate sources from a project-root aggregate `sources.yml` to per-entity `.yml` files under any `paths:` directory (`sources.md` Surface §"Filesystem layout", §"Source YAML shape"). Implement the `name:` override; reject `materialization:` on a source; address sources at `smelt.<path>`. Aggregate `sources.yml` is no longer recognised — its presence is a clear migration error.

**Pre-conditions.** Phases 1–5 done. Resolver classifies standalone YAMLs as sources (Phase 2).

**TDD tests to write first.**
- `crates/smelt-core/tests/source_yaml.rs::standalone_yml_loads_as_source` — `models/external/api/orders.yml` declaring columns produces a source addressable as `smelt.external.api.orders`.
- `crates/smelt-core/tests/source_yaml.rs::name_override_takes_precedence` — a source YAML with `name: legacy_db.orders_v2` resolves to `legacy_db.orders_v2` in `FROM`-clause emission, not `<target_schema>.<address-joined>`.
- `crates/smelt-core/tests/source_yaml.rs::materialization_on_source_is_error` — `materialization: table` (or any value) on a source YAML is a hard parse error with a message pointing to the seed sidecar shape.
- `crates/smelt-core/tests/source_yaml.rs::sidecar_takes_priority_over_source` — a YAML next to a same-stem CSV is a sidecar, not a source — even if the YAML would otherwise be valid as a source.
- `crates/smelt-core/tests/source_yaml.rs::aggregate_sources_yml_errors` — a project-root `sources.yml` produces a workspace-load error with a migration message naming the per-entity replacement layout.
- `crates/smelt-cli/tests/example_diagnostics.rs` — every example with sources (today: `examples/timeseries`, `examples/retail_analytics`, `examples/ecommerce`) migrates to per-entity files and produces zero diagnostics.

**Implementation shape.**
- Delete or repurpose `crates/smelt-core/src/sources.rs` aggregate-loader. Source discovery is now driven entirely by the resolver (Phase 2) — the standalone-YAML branch produces a `Source` entity whose schema comes from parsing that one file.
- Shared YAML-parsing function (used by both seed sidecars and source YAMLs) lives in `crates/smelt-db/src/schema.rs` per `seeds.md` References. A second function on top enforces the source-only constraints (`columns:` required; `materialization:` forbidden; `name:` allowed).
- `crates/smelt-db/src/code_actions.rs` — current actions edit the aggregate `sources.yml` (`yaml_edits.rs`). Update `generate_add_source_action` to write a new per-entity file at the spec'd address path; update `generate_add_column_action` to edit the resolved per-entity file. Drop the `yaml_edits.rs` line-scanner if no other consumer remains.
- Hard-cut migration: detect a project-root `sources.yml` at workspace load; emit a `WorkspaceLoadError::AggregateSourcesYmlNotSupported` with a one-line "split per-entity under `paths:`" message.
- Migrate `examples/*/sources.yml` to per-entity files under each project's `paths:` (likely `models/sources/...`).

**Critical files.**
- `crates/smelt-core/src/sources.rs` — removed or radically slimmed.
- `crates/smelt-core/src/{discovery.rs,project.rs}` — drop `is_sources_config_file` (`project.rs:105`).
- `crates/smelt-db/src/{schema.rs,code_actions.rs,yaml_edits.rs}` — code-action retargeting; consider deleting `yaml_edits.rs` outright.
- `examples/*/sources.yml` → `examples/*/<paths>/sources/...` (per project).

**Docs touched.**
- `docs/specs/sources.md` — Known Divergences §"Implementation lags spec": delete the "no per-entity source YAMLs" bullet.
- `docs-site/docs/guide/sources.md` — new (per spec References). Mirrors the user-visible portion of `sources.md`.
- `docs-site/docs/reference/sources-yml.md` — reconciled to the per-entity shape.
- `docs-site/docs/concepts/project-structure.md` — example workspace tree now shows per-entity source `.yml`s.

**Review checklist** (material findings only):
- [ ] Aggregate `sources.yml` produces a clear migration error.
- [ ] Per-entity sources resolve to addresses matching their workspace path.
- [ ] `name:` override emits the literal value in `FROM` clauses.
- [ ] `materialization:` on a source is rejected at parse time.
- [ ] Code actions write per-entity files at correct paths.
- [ ] Examples migrated; example_diagnostics is green.

**Commit.** `sources: per-entity YAMLs, name-override, drop aggregate sources.yml`

---

### Phase 7: LSP missing-sidecar diagnostic + "Pin schema" code action

**Goal.** Surface the `seeds.md` §"LSP integration" affordances: a workspace warning on any CSV without a sibling YAML, and a "Pin schema to sidecar YAML" code action that runs the inferencer and writes a sibling `.yml`. Hover for column descriptions on seeds and sources falls out of the unified schema path; verify it works.

**Pre-conditions.** Phases 1–6 done. Inferencer (Phase 4) and per-entity YAML grammar (Phases 5–6) in place.

**TDD tests to write first.**
- `crates/smelt-lsp/tests/missing_sidecar.rs::csv_without_sibling_yml_warns` — a workspace with one CSV and no YAML produces exactly one workspace-level warning per CSV with the spec'd message.
- `crates/smelt-lsp/tests/missing_sidecar.rs::adding_sibling_yml_clears_warning` — once a sidecar YAML is added, the warning disappears on the next analysis.
- `crates/smelt-lsp/tests/pin_schema_action.rs::action_writes_sidecar_yml` — invoking the "Pin schema to sidecar YAML" code action on a CSV produces a sibling `.yml` whose `columns:` match the inferencer's runtime output (the whole-file inference, not the 100-row sample, because the action knows the full file).
- `crates/smelt-lsp/tests/pin_schema_action.rs::action_includes_inferred_types` — the written YAML lists every column with `type:` and (where the inferencer has it) `nullable:`.
- `crates/smelt-lsp/tests/hover.rs::hover_on_source_column_returns_description` — hovering on `smelt.external.api.orders.user_id` in a model returns the `description:` from the source YAML.

**Implementation shape.**
- Diagnostic: extend `file_diagnostics()` (or the workspace-level diagnostic emitter) to walk every seed CSV and check for a sibling YAML. Warning code (e.g. `MissingSeedSidecar`).
- Code action: a new `CodeActionKind::PinSeedSchema` whose `apply` runs `infer_columns(...)` over the full CSV file and writes a `<stem>.yml` next to the CSV. If a sidecar already exists with mismatched columns, the action is *not* offered (re-pinning is deferred per `seeds.md` §"LSP integration").
- Hover: extend the existing column-hover path to consult the unified schema source (which now covers both seed sidecars and source YAMLs after Phase 6).

**Critical files.**
- `crates/smelt-lsp/src/lib.rs`.
- `crates/smelt-db/src/code_actions.rs`.
- `crates/smelt-core/src/seeds/infer.rs` — exposes the same inferencer used at runtime (no duplication).

**Docs touched.**
- `docs/specs/seeds.md` — Known Divergences §"Implementation lags spec": delete the bullets for missing-sidecar warning and "Pin schema" action.
- `docs-site/docs/guide/seeds.md` — short section on the LSP affordances and the recommended workflow ("CSV → save → `Pin schema` → commit").

**Review checklist** (material findings only):
- [ ] Warning is emitted exactly once per CSV without a sidecar.
- [ ] Code action is offered only when no sidecar exists.
- [ ] Pinned YAML round-trips through Phase 5's sidecar validator with no error.
- [ ] Hover works for both seed and source columns (shared schema path).

**Commit.** `lsp: missing-sidecar warning and Pin-schema code action for seeds`

---

### Phase 8: User-docs reconciliation and `cli.md` cross-ref

**Goal.** Make the `docs-site/` user docs match the migrated specs end-to-end. Update `cli.md` references so the related-specs list mentions `paths:` instead of `model_paths`/`seed_paths` (already noted in the spec diff but not yet in the spec body).

**Pre-conditions.** Phases 1–7 done. Implementation matches every Surface bullet across the five specs.

**TDD tests to write first.** This is a docs-only phase, but we still gate it on real coverage:
- `crates/smelt-cli/tests/example_diagnostics.rs` (run again as a regression gate).
- `docs-site/docs/guide/seeds.md`, `docs-site/docs/guide/sources.md`, `docs-site/docs/reference/smelt-yml.md`, `docs-site/docs/reference/sources-yml.md`, `docs-site/docs/concepts/project-structure.md` — manual review against the spec sections they mirror; the verification step (`/smelt:validate <slug>` for each spec) reports zero drift.
- `docs/specs/cli.md` References — line referring to `paths:` (replacing the legacy `model_paths`/`seed_paths` reference noted in `51384ca`).

**Implementation shape.**
- Walk every spec's References → User docs list and confirm each page exists, is up to date, and uses the same vocabulary. Any drift surfaces concrete edits in this phase.
- `cli.md` Surface §"Subcommand catalogue" — confirm `smelt seed` description still matches.
- Add a brief migration note in `docs-site/docs/concepts/project-structure.md` explaining the move from `model_paths`/`seed_paths`/`sources.yml` to `paths:` + per-entity files. (User-facing note; the migration is hard-cut so this is for projects upgrading.)

**Critical files.** None (docs only).

**Docs touched.**
- `docs/specs/cli.md` — References cross-ref update.
- `docs-site/docs/guide/seeds.md`, `docs-site/docs/guide/sources.md`.
- `docs-site/docs/reference/smelt-yml.md`, `docs-site/docs/reference/sources-yml.md`.
- `docs-site/docs/concepts/project-structure.md`.

**Review checklist** (material findings only):
- [ ] `/smelt:validate seeds`, `/smelt:validate sources`, `/smelt:validate architecture`, `/smelt:validate smelt_yml`, `/smelt:validate cli` all report zero drift.
- [ ] User docs use `paths:` everywhere; no remaining `model_paths` / `seed_paths` / aggregate `sources.yml` references.
- [ ] Migration note in user docs is one short paragraph, not a how-to-migrate guide (tooling is deferred).

**Commit.** `docs: reconcile user docs and cli spec to unified paths and per-entity yamls`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo build` and `cargo test` are green workspace-wide.
- `cargo test -p smelt-cli --test example_diagnostics` is green — every bundled example matches the new shape.
- `cargo test -p smelt-core` (resolver, inferencer, sidecar validator, source YAML), `cargo test -p smelt-backend-duckdb --test load_table`, `cargo test -p smelt-lsp` (missing-sidecar, pin-schema, hover) are green.
- Run `examples/timeseries` end-to-end: `smelt build` loads seeds via the new path, materialises models at `<schema>.<path-joined>`, and a downstream model that references an ephemeral seed compiles and executes.
- `/smelt:validate seeds`, `/smelt:validate sources`, `/smelt:validate architecture`, `/smelt:validate smelt_yml`, `/smelt:validate cli` each report zero drift.
- `git grep -nE 'model_paths|seed_paths|read_csv_auto|sources\.yml'` returns only intentional historical mentions (in `docs/plans/` history files and in this plan's "Deferred"-style references). No live code path matches.
