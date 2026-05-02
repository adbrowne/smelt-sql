# Plan: smelt.&lt;path&gt; universal addressing migration

**Date**: 2026-05-01
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md)
**Spec diff**: `24f9891` (architecture unified) + `a2a150b` (downstream specs aligned) + working-tree edit removing the "pre-implementation" Known Divergence
**Tracking PR / branch**: [#111](https://github.com/adbrowne/smelt-sql/pull/111) on `worktree-spec`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/architecture.md` — it is the correctness oracle. Surface §"Resolution: `smelt.<path>` is the universal addressing scheme", Surface §"Project layout", and Design §"Single addressing scheme `smelt.<path>` for all project-defined entities" govern every phase. Do not re-open settled spec decisions.
2. Confirm you are on the tracking branch. If not, ask the user before continuing.
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
- Honor architectural invariants from `CLAUDE.md` (`type_inference.rs` purity; `smelt-dialect` lightweight; sync core / async edges).

---

## Context

The architecture spec now mandates a single addressing scheme — `smelt.<path>` for value references and `smelt.<path>(args)` for calls — covering every project-defined entity (models, functions, seeds, sources, tests). The legacy `smelt.ref(...)`, `smelt.source(...)`, and `smelt.fn.<path>(...)` forms are retired; externs remain flat. This plan drives the parser, downstream consumers, examples, and user docs to that target. See architecture spec Surface §"Resolution" and Design §"Single addressing scheme" for the full rules.

## Scope

### In scope (spec coverage)
- Architecture Surface §"Resolution: `smelt.<path>` is the universal addressing scheme" — value form + call form, kind dispatch by file format/content, externs flat exception, kind-mismatch on `smelt.tests.*` in `TableExpr` position
- Architecture Surface §"Project layout" — workspace walk produces path-addressable entities regardless of directory naming
- Functions spec §Known Divergences — the "smelt.<path> call grammar is pre-implementation" callout is removed once the migration lands

### Explicitly deferred
- Namespace decoupled from directory path (architecture Known Divergences) — no `smelt.package` or per-directory aliases
- `smelt.test` assertion semantics — this plan introduces parsing of `smelt.test` declarations only insofar as the path resolver dispatches to them; assertion behaviour is `tests.md` future work
- LSP "QUALIFY will be rewritten" hints — separate divergence
- `smelt-check` extraction — separate divergence
- `smelt.metric()` — `functions.md` §10 keeps it out of scope

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 6e1963f | 2026-05-01 |
| 2a    | done     | 38628cf | 2026-05-01 |
| 2b    | done     | 8864f26 | 2026-05-01 |
| 2c    | done     | 59c7f16 | 2026-05-01 |
| 3     | done     | 19e412b | 2026-05-01 |
| 4     | done     | 7a7d320 | 2026-05-01 |
| 5a    | done     | a814f53 | 2026-05-02 |
| 5b    | done     | e90fc58 | 2026-05-02 |

---

### Phase 1: Parser foundation (additive)

**Goal.** Lex and parse the unified `smelt.<path>` value form and `smelt.<path>(args)` call form into a single AST surface, without breaking any existing legacy parse.

**Pre-conditions.** Tracking branch in place. Existing parser tests green.

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:
- `crates/smelt-parser/src/parser.rs::tests::parses_smelt_path_value_in_from` — `SELECT * FROM smelt.models.users` produces a single `SmeltPathRef` AST node with segments `["models", "users"]` in the `FROM` position.
- `crates/smelt-parser/src/parser.rs::tests::parses_smelt_path_value_in_argument_position` — `f(smelt.models.users)` produces a `SmeltPathRef` arg, distinct from a function call.
- `crates/smelt-parser/src/parser.rs::tests::parses_smelt_path_call_with_positional_args` — `smelt.functions.patterns.session_rollup(events, 30)` parses as a `SmeltPathCall` with two positional args.
- `crates/smelt-parser/src/parser.rs::tests::parses_smelt_path_call_with_named_args` — `smelt.models.margins(product_summary => smelt.models.product_summary)` parses with named-arg `=>` binding.
- `crates/smelt-parser/src/parser.rs::tests::smelt_path_call_supports_passing_clause` — path-call followed by `PASSING name AS (…)` parses (parity with current `smelt.fn.*` PASSING).
- `crates/smelt-parser/src/parser.rs::tests::legacy_smelt_ref_still_parses` — `smelt.ref('users')` continues to parse as a `FunctionCall` (legacy compat retained for Phase 1).
- `crates/smelt-parser-compat/tests/parse_equivalence.rs::pg_query_round_trip_smelt_path_in_from` — `SELECT * FROM smelt.models.users` round-trips through smelt-parser identically and pg_query rejects the smelt-extension SQL only after the standard normalize step (matching current `smelt.ref` handling).
- `crates/smelt-parser/tests/path_fixture.rs::test_workspace_path_form_parses` — a new minimal `.sql` file `examples/test_workspace/models/path_demo.sql` containing `SELECT * FROM smelt.models.users` parses with zero diagnostics.

**Implementation shape.**
- New `SyntaxKind` variants: `SMELT_PATH_REF` (value form, no parens) and `SMELT_PATH_CALL` (call form). Replace or co-exist with `SMELT_FN_CALL` — additive in this phase, removed in Phase 4.
- Lexer recognises the `smelt.` prefix and dotted-path segments uniformly; the parser disambiguates value vs. call by the trailing `(`.
- AST wrappers `SmeltPathRef { path: Vec<SyntaxToken> }` and `SmeltPathCall { path, args, passing }` in `crates/smelt-parser/src/ast.rs`.
- Legacy `RefCall::from_function_call`, `SourceCall::from_function_call`, and `SMELT_FN_CALL` paths untouched in this phase.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/{lexer.rs, parser.rs, syntax_kind.rs, ast.rs}` — add unified path grammar
- `crates/smelt-parser/src/symbol.rs` — extend cursor-context detection to recognise the new node
- `crates/smelt-parser-compat/src/{normalize.rs, gaps.rs}` — extend the smelt-extension stripper to handle path forms before pg_query / sqlparser sees the SQL
- `examples/test_workspace/models/path_demo.sql` — new minimal fixture
- `crates/smelt-parser-compat/tests/parse_equivalence.rs` — add path-form coverage

**Docs touched.**
- None in this phase. Spec already describes target surface; Phase 4 removes the `functions.md` Known Divergence callout once the migration is structurally complete.

**Review checklist** (material findings only):
- [ ] All eight TDD tests above exist and assert what's specified
- [ ] Legacy `smelt.ref`, `smelt.source`, `smelt.fn.<path>` parsing is unchanged (regression check via existing parser tests)
- [ ] Parser stays usable on invalid input (architecture Constraints §7) — no panics on partial path forms like `smelt.` or `smelt.models.`
- [ ] No consumer crate (`smelt-core`, `smelt-db`, `smelt-dialect`, `smelt-planner`, `smelt-cli`, `smelt-lsp`) is touched in this phase
- [ ] `examples/test_workspace/models/path_demo.sql` parses with zero diagnostics

**Commit.** `parser: add smelt.<path> grammar (additive)`

---

### Phase 2a: Data plane (smelt-core + smelt-db)

**Goal.** Make `smelt-core` ref extraction and `smelt-db` schema/type resolution consume `SmeltPathRef` / `SmeltPathCall` end-to-end, while continuing to recognise legacy nodes through a single adapter so the tree stays green.

**Pre-conditions.** Phase 1 done. New AST nodes available; legacy nodes still produced for legacy syntax.

**TDD tests to write first.**
- `crates/smelt-core/tests/path_refs.rs::extracts_path_refs_from_unified_ast` — given a parsed file with `FROM smelt.models.upstream`, `extract_refs` returns one entry whose path tuple is `("models", "upstream")` and whose kind is determined by the workspace file at `models/upstream.sql`.
- `crates/smelt-core/tests/path_refs.rs::extracts_path_refs_for_seed_and_source` — fixtures with `smelt.seeds.raw.users` and `smelt.sources.raw.events` resolve to `Seed` and `Source` kinds respectively.
- `crates/smelt-core/tests/path_refs.rs::path_refs_dependency_graph` — `DependencyGraph::build_from_workspace` on `examples/test_workspace/` (post-fixture) keys edges on path tuples.
- `crates/smelt-db/tests/path_resolution.rs::resolve_ref_path_form_returns_model_schema` — `resolve_ref(("models","users"))` returns the schema of `examples/test_workspace/models/users.sql`.
- `crates/smelt-db/tests/path_resolution.rs::resolve_ref_kind_mismatch_on_test_path` — referencing `smelt.tests.foo` in a `TableExpr` position emits a kind-mismatch diagnostic per architecture Surface §"Resolution".
- `crates/smelt-db/tests/path_resolution.rs::tableexpr_substitution_through_path_arg` — calling `smelt.functions.patterns.session_rollup(smelt.models.events, 30)` substitutes the `events` schema into the function body's `TypeContext`.
- `crates/smelt-db/tests/path_resolution.rs::legacy_smelt_ref_still_resolves` — `smelt.ref('users')` continues to resolve via the adapter (will be removed in Phase 4).

**Implementation shape.**
- `smelt-core/src/refs.rs`: a single `enum SmeltRef { Path(Vec<String>), LegacyRef(String), LegacySource(String), LegacyFn(Vec<String>) }` with a `to_path(&self, workspace: &Workspace) -> Vec<String>` adapter that maps every legacy form into the unified path tuple.
- `smelt-core/src/graph.rs`: `DependencyGraph` keyed on `Vec<String>` path tuples; legacy edges go through the adapter.
- `smelt-db/src/references.rs`: Salsa input query returns the unified path tuple; legacy nodes adapted at the boundary.
- `smelt-db/src/schema.rs` + `lib.rs`: `resolve_ref(path)` walks the workspace by path segments; dispatch on file format/content (bare SELECT → model schema; `.csv` → seed schema; `.yml` → source schema; `smelt.define` → function signature; `smelt.test` → kind-mismatch error if used in `TableExpr` position).
- `smelt-db/src/type_inference.rs`: `TableExpr` substitution accepts a path-form arg and produces the same row-polymorphic context as today's legacy form. **Stay pure** — no Salsa calls inside the inference functions (architecture Constraints §2).
- `smelt-db/src/backends.rs`: backend-namespace remap continues to work; no behavioural change.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/{refs.rs, graph.rs, discovery.rs}`
- `crates/smelt-db/src/{lib.rs, references.rs, schema.rs, type_inference.rs, backends.rs, tests.rs}`
- `crates/smelt-core/tests/path_refs.rs` (new)
- `crates/smelt-db/tests/path_resolution.rs` (new)
- `examples/test_workspace/` — small additions/edits required to support new fixture cases (e.g. a seed and a source for the kind-dispatch tests). Mechanical rewrite of all examples is Phase 3.

**Docs touched.**
- None — spec Surface already describes the target. User-doc updates are Phase 3.

**Review checklist** (material findings only):
- [ ] Path-tuple is the single internal key; no module branches on "is this a legacy node?"
- [ ] `type_inference.rs` purity holds (no Salsa imports added)
- [ ] All seven TDD tests above exist and assert what's specified
- [ ] No dialect / planner / cli / lsp file is touched in this phase
- [ ] Legacy-form integration tests still pass

**Commit.** `core+db: resolve smelt.<path> through unified pipeline`

---

### Phase 2b: Dialect printer

**Goal.** `smelt-dialect` rewrites unified `SmeltPathRef` and `SmeltPathCall` nodes to backend SQL, preserving DuckDB byte-identity for non-extension input.

**Pre-conditions.** Phases 1 and 2a done. Path-tuple kind dispatch available from `smelt-db`.

**TDD tests to write first.**
- `crates/smelt-dialect/tests/snapshots.rs::path_ref_to_model_emits_schema_qualified_name` — `SELECT * FROM smelt.models.users` → `SELECT * FROM <schema>.users` for the configured DuckDB schema.
- `crates/smelt-dialect/tests/snapshots.rs::path_ref_to_seed_emits_seed_table_name` — `smelt.seeds.raw.users` → the seed-load table identifier.
- `crates/smelt-dialect/tests/snapshots.rs::path_ref_to_source_emits_source_declared_name` — `smelt.sources.raw.events` → the `sources.yml`-declared identifier.
- `crates/smelt-dialect/tests/snapshots.rs::path_call_emits_expanded_function_body` — `smelt.functions.patterns.session_rollup(events, 30)` expands inline (re-parse-and-rewrite still works for path-form refs inside the body).
- `crates/smelt-dialect/tests/snapshots.rs::extern_call_unchanged` — `read_parquet('foo.parquet')` continues to emit verbatim (externs remain flat per architecture Surface §"Resolution").
- `crates/smelt-dialect/tests/snapshots.rs::duckdb_byte_identity_preserved_on_path_form` — a fixture combining path-form refs and ordinary DuckDB SQL round-trips byte-identically modulo the documented smelt-extension resolution rules.
- `crates/smelt-cli/tests/path_form_compile.rs::compiles_path_form_workspace_to_duckdb` — CLI compile of a small path-form workspace produces backend SQL equivalent to today's legacy-form output for the same logical models.

**Implementation shape.**
- `smelt-dialect/src/printer.rs`: a single match arm consumes `SmeltPathRef` / `SmeltPathCall`. The `Expander` closure (currently keyed on `SMELT_FN_CALL`) is rewired to dispatch on the path-tuple kind from Phase 2a; expansion logic is otherwise unchanged.
- `BackendCapabilities` and the cross-dialect rewrites (`EXPLODE`/`UNNEST`, `EVERY`/`BOOL_AND`, etc.) untouched.
- Legacy `RefCall` / `SourceCall` / `SMELT_FN_CALL` printer paths retained behind the same dispatch (removed in Phase 4).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-dialect/src/{printer.rs, lib.rs}`
- `crates/smelt-dialect/tests/snapshots.rs`
- `crates/smelt-cli/tests/path_form_compile.rs` (new) — narrow CLI smoke test only; full CLI consumer migration is Phase 2c

**Docs touched.**
- None.

**Review checklist** (material findings only):
- [ ] `smelt-dialect` retains its lightweight crate constraint (architecture Constraints §5) — no Arrow / Tokio / DuckDB pulled in
- [ ] DuckDB byte-identity property still holds (existing identity tests pass on the legacy fixtures)
- [ ] All seven TDD tests above exist and assert what's specified
- [ ] No `smelt-planner` / `smelt-lsp` / `smelt-cli` source file (other than the new smoke test) is touched in this phase

**Commit.** `dialect: emit backend SQL from smelt.<path> nodes`

---

### Phase 2c: Planner + CLI + LSP + bench

**Goal.** Every consumer that today pattern-matches on `SMELT_FN_CALL` / `RefCall` / `SourceCall` instead reads from the unified `SmeltPathRef` / `SmeltPathCall` surface.

**Pre-conditions.** Phases 1, 2a, 2b done.

**TDD tests to write first.**
- `crates/smelt-planner/tests/path_form_logical_plan.rs::function_call_path_resolves_to_logical_function_node` — a `smelt.<path>(...)` call whose path resolves to a `smelt.define` constructs a `FunctionCall { transparent: true, body: Some(_) }` node, and a path resolving to a model constructs the same shape with the model body inlined.
- `crates/smelt-planner/tests/path_form_logical_plan.rs::extern_call_constructs_blackbox_function_call` — bare-name extern call produces `FunctionCall { transparent: false, body: None }` (parity with today).
- `crates/smelt-cli/tests/path_form_e2e.rs::compile_and_plan_path_workspace` — end-to-end CLI run on a small path-form workspace produces the expected `Vec<Transformation>` and emits valid DuckDB SQL.
- `crates/smelt-cli/tests/build_summary_visibility.rs::path_form_models_appear_in_build_summary` — a path-form model is enumerated in CLI build output.
- `crates/smelt-lsp/tests/goto_definition.rs::goto_path_ref_jumps_to_owning_file` — goto-def on `smelt.models.users` jumps to `examples/test_workspace/models/users.sql`.
- `crates/smelt-lsp/tests/completion.rs::completes_workspace_paths_after_smelt_dot` — typing `smelt.` in a `FROM` position offers workspace path completions.
- `crates/smelt-lsp/tests/diagnostics.rs::test_path_in_tableexpr_position_diagnostic` — `FROM smelt.tests.foo` raises a kind-mismatch diagnostic at the path location.
- `crates/smelt-bench/tests/templates_emit_path_form.rs::sql_templates_emit_path_syntax` — generated synthetic SQL uses path syntax (no `smelt.ref(`/`smelt.fn.` substrings).

**Implementation shape.**
- `smelt-planner`: pattern-match on the unified node; rule registry untouched. Logical-plan construction reads the path tuple via the Phase 2a adapter.
- `smelt-cli`: compiler, `commands/run.rs`, `commands/backbuild.rs`, and `test_compiler.rs` consume unified nodes through the same APIs `smelt-db` exposes.
- `smelt-lsp`: completion provider walks the workspace and offers paths under `smelt.`; goto-def uses the path-tuple resolver from Phase 2a; diagnostics surface the kind-mismatch error.
- `smelt-bench`: `model_gen/{sql_templates,python_templates}.rs` and `harness/parser_bench.rs` emit path syntax in synthetic models.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-planner/src/{graph.rs, analysis/mod.rs, lib.rs}`
- `crates/smelt-cli/src/{compiler.rs, test_compiler.rs, commands/{run.rs, backbuild.rs}}`
- `crates/smelt-lsp/src/{lib.rs, completion.rs, goto_definition.rs, diagnostics.rs}` (file names follow current layout)
- `crates/smelt-bench/src/{harness/parser_bench.rs, model_gen/{sql_templates,python_templates}.rs}`, `crates/smelt-bench/benches/parser_throughput.rs`
- New tests listed under TDD above

**Docs touched.**
- None — Phase 3 owns user-doc rewrites. Code-level comments referencing legacy forms may be updated in passing.

**Review checklist** (material findings only):
- [ ] Every consumer reads from the unified node surface; no consumer branches on "legacy vs unified"
- [ ] `cargo test -p smelt-cli --test example_diagnostics` still green (legacy examples haven't been migrated yet — Phase 3 — so the suite must still tolerate both forms via the adapter)
- [ ] All eight TDD tests above exist and assert what's specified
- [ ] LSP completion respects workspace boundaries (no leaked filesystem paths outside the workspace)
- [ ] Sync core / async edges constraint preserved (architecture Constraints §4) — no new async added inside `smelt-planner`

**Commit.** `planner+cli+lsp+bench: consume unified smelt.<path> AST`

---

### Phase 3: Examples and user-doc migration

**Goal.** Every committed `.sql` / `.yml` / `.md` file under `examples/` and `docs-site/` uses the unified `smelt.<path>` syntax. No production source touches the legacy forms.

**Pre-conditions.** Phases 1–2c done. Adapter still resolves legacy forms (will be deleted in Phase 4).

**TDD tests to write first.**
- `crates/smelt-cli/tests/example_diagnostics.rs::all_examples_use_path_syntax` — fail if any `examples/**/*.sql` file contains `smelt.ref(`, `smelt.source(`, or `smelt.fn.` (string scan).
- `crates/smelt-cli/tests/example_diagnostics.rs::all_examples_have_zero_lsp_diagnostics_after_migration` — extend the existing diagnostic suite to assert that every workspace under `examples/` produces zero diagnostics with the new syntax.
- `tools/migrate_smelt_path/tests/rewrite.rs::rewrites_smelt_ref_to_path_form` — given `SELECT * FROM smelt.ref('users')` against a workspace where `users` is `models/users.sql`, rewrite emits `SELECT * FROM smelt.models.users`.
- `tools/migrate_smelt_path/tests/rewrite.rs::rewrites_smelt_source_to_path_form` — `smelt.source('raw.events')` → `smelt.sources.raw.events`.
- `tools/migrate_smelt_path/tests/rewrite.rs::rewrites_smelt_fn_to_path_form` — `smelt.fn.core.safe_divide(x, y)` → `smelt.functions.core.safe_divide(x, y)` (or whatever the workspace path resolves to).

**Implementation shape.**
- New `tools/migrate_smelt_path/` Rust binary (or one-off subcommand under `smelt-cli` gated by `--internal-migrate`) that:
  1. Loads a workspace, builds the path-tuple index from `smelt-db` discovery.
  2. Walks every `.sql` file, parses with `smelt-parser`, locates legacy forms, rewrites in place using token-level edits (preserves comments and whitespace).
  3. Mechanically rewrites all 11 example workspaces (`broken`, `demo_workspace`, `ecommerce`, `ephemeral_demo`, `functions_demo`, `huge`, `multi_engine`, `retail_analytics`, `smelt_shop_min`, `test_workspace`, `timeseries`).
- User-doc rewrites under `docs-site/` are mechanical search-and-replace per workspace conventions; cite the migrated example workspaces.
- Spec cleanup: remove `functions.md` Known Divergence bullet (line ~217) "smelt.<path> call grammar is pre-implementation".

**Critical files (allowed to touch in this phase).**
- `tools/migrate_smelt_path/` (new throwaway binary; deleted in Phase 4)
- `examples/**/*.sql`, `examples/**/*.yml` — mechanical rewrite output
- `docs-site/docs/concepts/{how-it-works,project-structure}.md`
- `docs-site/docs/getting-started/quickstart.md`
- `docs-site/docs/guide/*.md` (sql-models, sources, seeds, functions, materializations, incremental-models, schema-evolution, testing, targets, datagen, python-models, editor-features, editor-setup)
- `docs-site/docs/reference/{language, sources-yml}.md`
- `docs-site/docs/developing/architecture.md`, `docs-site/docs/index.md`
- `docs/specs/functions.md` — remove the "smelt.<path> call grammar is pre-implementation" Known Divergence

**Docs touched.**
- All user-doc migrations listed above
- `docs/specs/functions.md` Known Divergences section pruned

**Review checklist** (material findings only):
- [ ] Zero matches for `smelt\.ref\(|smelt\.source\(|smelt\.fn\.` under `examples/` and `docs-site/`
- [ ] `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics across every example workspace
- [ ] User-doc snippets compile (path forms used in code blocks parse cleanly when run through `smelt-parser`)
- [ ] No production source code touched (parser/db/dialect/planner/cli/lsp/bench unchanged this phase)
- [ ] Spec footer "smelt.<path> call grammar is pre-implementation" callout removed from `functions.md`

**Commit.** `examples+docs: migrate to smelt.<path> syntax`

---

### Phase 4: Legacy parser removal

**Goal.** Delete the legacy `smelt.ref(...)`, `smelt.source(...)`, and `smelt.fn.<path>(...)` parser/AST/printer paths and the Phase 2a adapter that bridged them. After this phase, the only way to reference a project-defined entity is `smelt.<path>`.

**Pre-conditions.** Phases 1–3 done. Tree contains zero legacy-form usage outside test fixtures specifically asserting the legacy form is rejected.

**TDD tests to write first.**
- `crates/smelt-parser/src/parser.rs::tests::legacy_smelt_ref_now_rejected` — `SELECT * FROM smelt.ref('users')` produces a parse error pointing the user at `smelt.<path>`.
- `crates/smelt-parser/src/parser.rs::tests::legacy_smelt_source_now_rejected` — `smelt.source('raw.events')` produces a parse error.
- `crates/smelt-parser/src/parser.rs::tests::legacy_smelt_fn_call_now_rejected` — `smelt.fn.core.safe_divide(x, y)` produces a parse error.
- `crates/smelt-parser/tests/no_legacy_ast_nodes.rs::ast_module_has_no_legacy_projections` — `RefCall`, `SourceCall`, and `SMELT_FN_CALL` are not present in the public AST surface.
- `crates/smelt-parser-compat/tests/parse_equivalence.rs::gaps_registry_drops_legacy_only_entries` — gap-registry entries that existed only because of legacy syntax are removed; remaining entries reference only the unified surface.
- `crates/smelt-cli/tests/example_diagnostics.rs::all_examples_clean_after_legacy_removal` — full example suite still green with the adapter gone.

**Implementation shape.**
- Delete `RefCall::from_function_call`, `SourceCall::from_function_call`, the `SMELT_FN_CALL` SyntaxKind, and the parser arms that produced them.
- Delete `SmeltRef::LegacyRef` / `LegacySource` / `LegacyFn` adapter variants from `smelt-core/src/refs.rs`; `SmeltRef` becomes `Vec<String>` outright.
- Delete legacy printer arms from `smelt-dialect/src/printer.rs`.
- Delete LSP cursor-context legacy detection in `smelt-parser/src/symbol.rs`.
- Trim `smelt-parser-compat/src/gaps.rs` of legacy-form entries.
- Delete the `tools/migrate_smelt_path/` binary from Phase 3.
- Final spec-anchored sweep: ensure `docs/specs/architecture.md` "Known Divergences" section no longer mentions the migration (already removed in the spec edit that triggered this plan; verify on review).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/{lexer.rs, parser.rs, syntax_kind.rs, ast.rs, symbol.rs}`
- `crates/smelt-core/src/refs.rs`
- `crates/smelt-db/src/{references.rs, schema.rs}`
- `crates/smelt-dialect/src/printer.rs`
- `crates/smelt-parser-compat/src/{normalize.rs, gaps.rs}`, `crates/smelt-parser-compat/tests/parse_equivalence.rs`
- `tools/migrate_smelt_path/` (delete)

**Docs touched.**
- None — spec was the trigger; docs were rewritten in Phase 3.

**Review checklist** (material findings only):
- [ ] No reference to `RefCall`, `SourceCall`, `SMELT_FN_CALL`, `smelt.ref`, `smelt.source`, or `smelt.fn.` remains in `crates/`
- [ ] `cargo test` and `cargo clippy --all-targets` clean across the workspace
- [ ] All six TDD tests above exist and assert what's specified
- [ ] `tools/migrate_smelt_path/` is deleted (or deliberately preserved as a documented utility — flag the choice in Deferred)
- [ ] `/smelt:validate architecture` reports zero drift

**Commit.** `parser: drop legacy smelt.ref/source/fn forms`

---

### Phase 5a: Port function-call diagnostics to SmeltPathCall

**Goal.** Extend `smelt_fn_call_diagnostics_for_file` to walk `SMELT_PATH_CALL` nodes and produce identical diagnostics (`ArgTypeMismatch`, `MissingArgument`, `UnknownSmeltFn`, body-type errors, cycle errors, etc.) for `smelt.functions.<name>(...)` calls. After this phase the same checks fire regardless of whether the call uses the legacy `smelt.fn.*` node or the path `smelt.functions.*` node. The `broken/` fixtures are still written in `smelt.fn.*` form; migration is Phase 5b.

**Pre-conditions.** Phases 1–4 done. `SmeltPathCall` AST node exists. `SmeltFnCall` still parsed and checked.

**TDD tests to write first.**
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_arg_type_mismatch` — a workspace containing `smelt.define needs_number(x: Expr<Numeric>) AS (x + 1)` and a model `SELECT smelt.functions.needs_number('text') AS r` produces exactly one `ArgTypeMismatch` diagnostic anchored at the `'text'` argument span. Mirrors the existing `fn_call_wrong_arg_type.sql` fixture behaviour.
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_missing_arg` — `SELECT smelt.functions.takes_two(1) AS r` (where `takes_two(a, b)` is defined) produces exactly one `MissingArgument` diagnostic.
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_unknown_fn` — `SELECT smelt.functions.does_not_exist(1) AS r` produces exactly one `UnknownSmeltFn` diagnostic anchored at the path span.
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_nested_body_error` — a call through two levels of transparent functions (`outer_call(y) → middle(z) → inner_unary(z)`) where `inner_unary` has a body type error: calling `smelt.functions.outer_call(1)` produces a diagnostic with a frame stack reaching into the inner body, identical to the `fn_nested_call_error.sql` fixture.
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_tableexpr_row_requirement` — calling a `TableExpr<{revenue: Numeric, cost: Numeric}>` function with a table that is missing `cost` produces `RowRequirementMissing`. Mirrors `fn_row_requirement_missing.sql`.
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs::path_form_parity` — for each diagnostic code already exercised by a `smelt.fn.*` fixture in `examples/broken/`, calling the same function via `smelt.functions.*` form against the same workspace produces the same set of diagnostic codes (codes only, not spans — spans may differ by prefix length). This is the parity test; it blocks migration in 5b on any code gap.

**Implementation shape.**
- Add `pub fn check_smelt_path_call(call: &SmeltPathCall, ctx: &TypeContext, text: &str, ...)` to `crates/smelt-db/src/function_body_check.rs`. Its signature mirrors `check_smelt_fn_call` exactly (same callback parameters); the only difference is the input node type. Internally derive `fn_id` as `call.segments().last()` (same last-segment rule as `SmeltFnCall`). Derive `path_range` from `call.text_range()` (the whole node) as the fallback; add a `call_path_range() -> Option<TextRange>` accessor to `SmeltPathCall` in `ast.rs` that returns the `SmeltPath` child's range, so diagnostics anchor to the path not the parens.
- In `smelt_fn_call_diagnostics_for_file` (lib.rs): after collecting `SMELT_FN_CALL` nodes, collect `SMELT_PATH_CALL` nodes and call `check_smelt_path_call` for each one using the same closures built earlier in the function. Extend the `nested_handler` inside `check_function_body_select` to also walk `SMELT_PATH_CALL` children.
- **Stay pure** (architecture Constraints §2): no new Salsa calls inside `check_smelt_path_call`; closures are passed in from the Salsa boundary exactly as for `check_smelt_fn_call`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/function_body_check.rs` — add `check_smelt_path_call`, extend nested handler
- `crates/smelt-db/src/lib.rs` — extend `smelt_fn_call_diagnostics_for_file` walker
- `crates/smelt-parser/src/ast.rs` — add `call_path_range()` to `SmeltPathCall`
- `crates/smelt-db/tests/fn_path_call_diagnostics.rs` (new)

**Docs touched.**
- None in this phase.

**Review checklist** (material findings only):
- [ ] All six TDD tests above exist and are green
- [ ] All existing `broken/` fixture diagnostics still fire (no regression in `smelt.fn.*` checking)
- [ ] `check_smelt_path_call` uses the same closure callbacks as `check_smelt_fn_call`; no Salsa calls inside the pure checker
- [ ] `path_form_parity` test covers every diagnostic code exercised by the `broken/` `smelt.fn.*` fixtures
- [ ] `cargo clippy --all-targets` zero warnings

**Commit.** `db: extend function-call diagnostics to smelt.<path> call form`

---

### Phase 5b: Migrate broken/ fixtures and complete smelt.fn.* removal

**Goal.** Migrate every `broken/` fixture from `smelt.fn.*` to `smelt.functions.*`, delete the `SmeltFnCall` / `SMELT_FN_CALL` AST surface, remove `SmeltRef::LegacyFn`, retire the `smelt.fn.` parser arm, and drop the `broken/` skip from `all_examples_use_path_syntax`. After this phase `smelt.fn.*` is completely gone from the codebase.

**Pre-conditions.** Phase 5a done. `check_smelt_path_call` produces correct diagnostics for all broken-fixture codes.

**TDD tests to write first.**
- `crates/smelt-parser/src/parser.rs::tests::smelt_fn_form_rejected_by_parser` — `smelt.fn.foo(x)` produces a parse error with a suggestion pointing the user at `smelt.functions.foo(x)` (or general `smelt.<path>` form).
- `crates/smelt-parser/tests/no_legacy_fn_ast.rs::smelt_fn_call_not_in_public_ast` — `SmeltFnCall` is not a public type in `smelt_parser::ast`; attempting to use it by name fails to compile. (Implement as a doc-test or compile-error test using `trybuild`, or simply assert the type no longer exists in the module's exported names via reflection.)
- `crates/smelt-cli/tests/example_diagnostics.rs::all_examples_use_path_syntax_including_broken` — the existing `all_examples_use_path_syntax` test passes with the `broken/` skip removed; i.e. the migrated `broken/` fixtures contain no `smelt.ref(`, `smelt.source(`, or `smelt.fn.` strings.
- `crates/smelt-cli/tests/example_diagnostics.rs::broken_workspace_diagnostics_still_fire` — every expected diagnostic code from the broken workspace (verified against a reference list in the test) still fires after the migration, confirming Phase 5a's parity guarantee holds end-to-end on the migrated files.

**Implementation shape.**
- Rewrite every `examples/broken/models/*.sql` file that uses `smelt.fn.X(...)` → `smelt.functions.X(...)`. The directory structure and function declarations (`smelt.define`, `smelt.extern`) are unchanged; only call sites change.
- Remove `SmeltRef::LegacyFn` from `crates/smelt-core/src/refs.rs` (including the `to_path` adapter arm and `fn_id` arm).
- Delete `SmeltFnCall` struct and `SMELT_FN_CALL` `SyntaxKind` variant; remove the parser arm in `smelt-parser` that recognises `smelt.fn.` as a function-call prefix.
- Delete `check_smelt_fn_call` from `function_body_check.rs`; the `SMELT_FN_CALL` walker in `smelt_fn_call_diagnostics_for_file` is removed (only the `SMELT_PATH_CALL` walker remains).
- Remove the `broken/` exclusion comment and `if ... == "broken"` guard from `all_examples_use_path_syntax`.
- Update `docs/specs/functions.md` Known Divergences: remove the bullet "`smelt.<path>` function call diagnostics cover only `smelt.fn.*` form" (this divergence is now resolved).

**Critical files (allowed to touch in this phase).**
- `examples/broken/models/*.sql` — migrate all `smelt.fn.*` call sites
- `crates/smelt-parser/src/{lexer.rs, parser.rs, syntax_kind.rs, ast.rs}` — remove `SmeltFnCall` / `SMELT_FN_CALL`
- `crates/smelt-core/src/refs.rs` — remove `LegacyFn`
- `crates/smelt-db/src/{function_body_check.rs, lib.rs, references.rs}` — remove `check_smelt_fn_call` and `SMELT_FN_CALL` walker
- `crates/smelt-cli/tests/example_diagnostics.rs` — remove `broken/` skip
- `docs/specs/functions.md` — remove resolved Known Divergence bullet

**Docs touched.**
- `docs/specs/functions.md` Known Divergences section updated

**Review checklist** (material findings only):
- [ ] Zero matches for `smelt\.fn\.` in `crates/` and `examples/` (excluding string literals in rejection-error test assertions)
- [ ] `SmeltFnCall` and `SMELT_FN_CALL` do not appear in any non-test source file
- [ ] `SmeltRef::LegacyFn` is gone from `smelt-core`
- [ ] `broken_workspace_diagnostics_still_fire` test covers every expected diagnostic code
- [ ] All four TDD tests above exist and are green
- [ ] `cargo test` and `cargo clippy --all-targets` clean

**Commit.** `parser+db+core: complete smelt.fn.* removal`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **`examples/broken/` not migrated to path form** (Phase 3). The `broken/` workspace fixtures use `smelt.fn.*` calls to trigger specific function type diagnostics (`ArgTypeMismatch`, `UnknownIdentifier` with expansion frames). The `smelt_fn_call_diagnostics_for_file` type checker only operates on `SmeltFnCall` nodes, not yet `SmeltPathCall`. Migrating `broken/` would silence these diagnostics. Fix deferred to a follow-on phase that extends function-call type checking to handle `SmeltPathCall`; the `all_examples_use_path_syntax` test skips `broken/` with an explanatory comment. Tracked in `docs/specs/functions.md` Known Divergences.
- **`SMELT_PATH_REF` trailing-trivia printer bug fixed in Phase 3** (smelt-dialect). The parser's `skip_trivia()` look-ahead before `start_node_at(outer_checkpoint, SMELT_PATH_REF)` wrapped trailing whitespace inside the node. The printer then emitted the resolved SQL without that whitespace, collapsing `raw.orders AS o` → `raw.ordersAS o`. Fixed in `printer.rs` by re-emitting trailing trivia tokens after the resolver replacement.
- **Path resolver only matched `[ns, name]` (two-segment models)**. `smelt.models.staging.stg_events` (three segments) returned `None` from `make_path_ref_resolver`, causing DuckDB `NameListToString` errors. Fixed by changing the pattern to `[ns, rest @ ..] if ns == "models"` and using `rest.last()` as the physical table name.
- **`smelt.fn.*` / `SmeltFnCall` removal deferred** (Phase 4). The `examples/broken/` workspace still uses `smelt.fn.*` calls to exercise `ArgTypeMismatch`, `UnknownIdentifier` with expansion frames, and other function-body diagnostics. The `smelt_fn_call_diagnostics_for_file` type checker operates on `SmeltFnCall` nodes only; removing `SMELT_FN_CALL`/`SmeltFnCall` would silence those diagnostics. `SmeltRef::LegacyFn` is kept in `smelt-core` for now. Full removal requires extending function-call type checking to `SmeltPathCall` first, then migrating `broken/` fixtures to path form.
- **`workspace_function_call_graph` blind to `SMELT_PATH_CALL` edges** (Phase 5a review). The call-graph builder that powers cycle detection (`function_call_cycle_fn_ids`) only walks `SMELT_FN_CALL` nodes inside `smelt.define` bodies. A body containing `smelt.functions.bar(x)` (SMELT_PATH_CALL) is invisible to the cycle detector. No current fixtures expose this gap — broken/ bodies still use `smelt.fn.*`. After Phase 5b migrates broken/ bodies to `smelt.functions.*`, cycle detection will break for those fixtures unless the call-graph walker is extended. Must be fixed in Phase 5b before removing `SMELT_FN_CALL`.
- **`walk_body` Expression-shape bodies do not dispatch `SMELT_PATH_CALL` nested calls** (Phase 5a review). `check_function_body_select` was extended to walk `SMELT_PATH_CALL` nodes, but the Expression-shape body walk (`walk_body_with_ctx`) only dispatches `SMELT_FN_CALL`. A scalar body like `AS (smelt.functions.bar(x) + 1)` calling further path-form functions silently skips the nested diagnostic check. Current fixtures are unaffected because broken/ bodies still use `smelt.fn.*`. After Phase 5b migration, nested path calls inside scalar bodies will be missed. Must be fixed in Phase 5b.

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test` workspace-wide
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo clippy --all-targets` zero warnings
- `cargo run -p smelt-cli -- run --workspace examples/retail_analytics` succeeds end-to-end
- `/smelt:validate architecture` reports zero drift
- `/smelt:validate functions` reports zero drift (Known Divergence for path-call diagnostics removed)
- `grep -r "smelt\.ref(\|smelt\.source(\|smelt\.fn\." crates/ examples/ docs-site/ docs/specs/` returns no matches except string literals inside rejection-error test assertions
