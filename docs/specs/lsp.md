---
feature: lsp
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# LSP

> **What this is.** A normative spec for the smelt Language Server Protocol implementation — the feature matrix, diagnostic categories and severity, go-to-definition scope, rename scope, and completion behavior. Out of scope: the diagnostic-code catalogue (see `diagnostics.md`); workspace loading and project isolation (see `architecture.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### Capabilities

The smelt LSP server registers the following capabilities:

| Feature | Status |
|---------|--------|
| Diagnostics (push) | ✓ |
| Go-to-Definition | ✓ |
| Find References | ✓ |
| Hover | ✓ |
| Completions | ✓ (trigger chars: `'`, `(`, `.`) |
| Rename | ✓ |
| Prepare Rename | ✓ |
| Code Actions | ✓ |
| Code Lens | see `property_diff.md` §"Editor" |

Text document synchronization is **full** (not incremental). Each file change sends the complete new text.

### Editor setup

The LSP server binary is `smelt-lsp`. Editors connect to it via stdio. It is language-server-compatible (tower-lsp) and integrates with any editor supporting LSP, including VS Code (via the official extension), Neovim, and Helix.

The VS Code extension auto-activates when a `*.sql` file is found under a `models/` directory (`workspaceContains:**/models/**/*.sql`). It sets the server's working directory to the project root.

### Watched files

The server watches, via `workspace/didChangeWatchedFiles`, the files that participate in the loaded project. The watch set is **derived from discovery, not hardcoded by kind**: smelt discovers entities by walking every non-excluded subdirectory under the project root, and *any* `.sql` file is discoverable unless ruled out by the project's exclude set (`architecture.md` §"Resolution" — discovery is project-wide; `paths:` only strips address prefixes, it does not gate discovery). The server therefore watches:

- **All project `.sql` files** in non-excluded directories — model definitions and `smelt.define` function definitions alike, wherever they live (defines and models are not confined to `functions/` or `models/`). External edits (e.g. `git checkout`, `sed`) that bypass `textDocument/didChange` are picked up so dependent models re-diagnose.
- **Python model files** (`.py`) in non-excluded directories. Changes trigger a workspace refresh: re-discovering Python models and re-running type inference.

The watcher follows the project-wide-discovery + exclude rule rather than hardcoded `**/models/**` / `**/functions/**` globs; when a loaded project narrows discovery via `paths:`/exclude config, the watch set narrows with it.

## Semantics

### Diagnostics

`PropertyDowngrade` is an editor-only diagnostic derived from a git-baseline property diff rather
than from a single file's Salsa queries; see `property_diff.md` §"Editor" and §"Diagnostics" for
its trigger, anchoring, and the baseline-watch rule.

On every file change the server republishes diagnostics for **the changed file plus every file whose Salsa-derived diagnostics changed as a result** — at minimum all open files in the same project, since an upstream edit can stale a downstream file's diagnostics. Publishing only the changed file would leave consumers of the edited entity showing diagnostics computed against the old text. All diagnostics for a file are derived from the Salsa incremental computation database; only affected queries are re-run, so the set of files whose diagnostics actually changed is exactly the set Salsa recomputes.

#### Severity levels

Each diagnostic's severity is owned by the `diagnostics.md` catalogue, not re-declared here. The LSP only maps the catalogue severity onto an LSP level:

| Severity (from `diagnostics.md`) | LSP Level |
|----------|-----------|
| Error | `ERROR` |
| Warning | `WARNING` |
| Info | `INFORMATION` |
| Hint | `HINT` |

The LSP must not assign a code a different severity than the catalogue gives it — a code's Error/Warning status is load-bearing (the CLI build gates on `Error`), so a single source of truth keeps the two surfaces consistent.

#### Diagnostic categories

**Parse and syntax:**
- `ParseError` — SQL syntax error
- `UnsupportedConstruct` — construct that parses but is not supported (e.g., PIVOT)
- `YamlParseError` — invalid YAML in frontmatter or a per-entity source / seed-sidecar `.yml`
- `FrontmatterParseError` — malformed YAML frontmatter block
- `InvalidModel` — model file cannot be parsed at all

**References:**
- `UndefinedModelRef` / `UndefinedSource` — a `smelt.<path>` reference in value position does not resolve to any project entity. `UndefinedModelRef` is the default for a bare unresolved `smelt.<path>` (the intended kind is unknowable when nothing exists at the path); `UndefinedSource` is reserved for a reference that resolves to a *source* declaration that is itself missing or invalid. (For a call form `smelt.<path>(...)` that resolves to no function, `UnknownSmeltFn` fires — see `functions.md`.) The diagnostic message is kind-aware on the *expected* kind at the use site (a missing `FROM smelt.<path>` referent reports "model, seed, or source") so user-facing messages stay specific.
- `KindMismatch` — entity used in the wrong context (e.g., a test model in a FROM clause)
- `CircularDependency` — model dependency graph contains a cycle
- `CteCycle` — CTE references itself directly or transitively
- `CteShadowsCallerCte` — a transparent function body declares a CTE whose name collides with a CTE in the calling model; callers must rename one CTE to resolve the ambiguity

**Columns:**
- `UndeclaredColumn` — column reference not found in upstream schema or declared sources
- `AmbiguousColumn` — unqualified column name matches multiple upstream sources

**Types:**
- `CannotInferType` — type inference cannot determine the type of an expression
- `TypeMismatch` — operation applied to incompatible types
- `UnknownCastType` — CAST target type is not recognized
- `SourceTypeError` — invalid type string in a source `.yml` (owned by `sources.md`; mirrored here for completeness)

**Functions (`smelt.define`, `smelt.extern`):**
- `UnrecognizedFunction` — call to an SQL built-in function not in the recognized registry
- `UnknownSmeltFn` — a `smelt.<path>(…)` call that resolves to no declared `smelt.define` in the project
- `DuplicateFunctionDefinition` — two `smelt.define` blocks with the same name
- `InvalidFunctionTypeRef` — malformed type annotation
- `FunctionBodyTypeMismatch` — type error inside function body
- `UnknownIdentifier` — undefined variable in function body
- `DuplicateParameterName` — parameter name collision
- `MissingArgument` — required parameter omitted in call
- `ArgTypeMismatch` — argument type does not satisfy parameter constraint
- `ExternCollidesWithBuiltin` — `smelt.extern` shadows a built-in function
- `BackendsWideningNotAllowed` — declared backends broader than parent context
- `WindowInScalarContext` — window function used in WHERE or GROUP BY
- `ParameterShadowsColumn` — parameter name shadows an in-scope column
- `RowRequirementUnsatisfied` — `TableExpr<{...}>` schema constraint not met
- `UnknownContext` — undefined context identifier in `Expr<T, ctx>`
- `ContextMismatch` — annotation disagrees with inferred context
- `ReturnTypeMismatch` — return type annotation doesn't match inferred type
- `UnknownPassingParameter` — PASSING clause references undefined parameter
- `FunctionCallCycle` — transparent function call graph contains a cycle

**Wide reflection (`smelt.models.*`, `smelt.sources.*`, `ModelRef`, `SourceRef`):**
- `WithTagRequiresText` — `with_tag` argument is not a compile-time `Text` literal
- `WithTagNamedArgument` — `with_tag` argument supplied as a named argument instead of positional
- `WideReflectionUnknownAccessor` — accessor name after `smelt.models.` or `smelt.sources.` is not in the closed set `{with_tag, all}`
- `WideReflectionUnexpectedArgument` — `all` accessor received an argument
- `ModelRefFieldUnknown` — field name in a `ModelRef` field projection is not in the closed set `{path, name, tags, columns}`
- `SourceRefFieldUnknown` — field name in a `SourceRef` field projection is not in the closed set `{path, name, tags, columns}`

**Generator files (`generates: models`):**
- `GeneratesUnknownValue` — `generates:` value other than `models`
- `GeneratesMixedWithBareModel` — `generates: models` combined with a `name:` field or Layer-1 delimiters
- `GenerateFileBareSelectForbidden` — generator file body contains a top-level bare `SELECT` / `WITH` / `VALUES`
- `GenerateFileBodyTypeError` — generator file body does not synthesise `List<ModelDef>`
- `ModelDefOutsideGeneratorFile` — `ModelDef { … }` literal in a non-generator-file context
- `ModelDefInvalidName` — `ModelDef.name` is empty or contains non-path-safe characters
- `ModelDefInvalidMaterialization` — `ModelDef.materialization` is not in `{'view', 'table', 'incremental'}`
- `ModelDefDuplicateName` — two `ModelDef` values in the same generator emit with the same `name`
- `ModelDefHandAuthoredCollision` — generator-emitted path collides with a hand-authored model or another generator's emission
- `GeneratorBodyForbidsModelReflection` — a generator's body invokes `smelt.models.with_tag` or `smelt.models.all`

Diagnostics surfacing from inside a generator body's HOF chain carry a **`<generator>` frame** as the outermost frame in the diagnostic's frame stack. The frame has `function = "<generator>"`, `fn_id = None`, `call_site_range` = the generator file's body range, and an optional `model_origin` = the offending `ModelDef.name` value-expression range. This frame stacks atop any HOF anonymous frames and loader `map_origin` / `model_origin` / `source_origin` provenance per the `expansion.md` anonymous-frame contract.

**Other:**
- `UnstableSchemaRequired` — `provenance:` used without `unstable_schema: true`
- `AsStructUnsupportedBackend` — `smelt.as_struct()` called on unsupported backend
- `ProvenanceMismatch` — declared vs. actual column reads disagree
- `JoinsMismatch` — declared join not present in FROM clause
- `MissingProvenancePushdownAdvisory` (Hint) — suggests declaring provenance
- `MalformedSource` — malformed entry in a source `.yml` (owned by `sources.md`; mirrored here for completeness)

### Go-to-Definition

Go-to-Definition resolves the following identifier types:

| Identifier | Resolves to |
|------------|-------------|
| `smelt.<path>` (resolves to a model) | The SQL (or Python) source file for that model |
| `smelt.<path>` (resolves to a source) | The source `.yml` file declaring that table |
| `smelt.<path>` (resolves to a seed) | The seed `.csv` file (or the sidecar `.yml` if cursor is on a column) |
| `smelt.<path>(...)` (resolves to a function) | The `smelt.define` declaration |
| CTE name (reference site) | The CTE definition in the same file's WITH clause |
| Table alias (in column reference) | The FROM or JOIN clause that defines the alias |
| Column reference (qualified) | The upstream column definition, traced through SELECT chains |
| Column reference (unqualified, unambiguous) | The upstream column definition |
| Column reference (unqualified, ambiguous) | All matching upstream definitions (array response) |
| Python `@model` function (from SQL ref) | The `.py` file, at the decorator line |
| `smelt.columns_of` call path | Reference page (URL hint, graceful no-op when client lacks support) |
| Meta-`Text` lifted as identifier (statically traceable) | The source column's declaration in the upstream model, source, or seed; graceful no-op when the column origin cannot be traced |
| `smelt.models.*` / `smelt.sources.*` accessor call path | Reference page (URL hint, graceful no-op when client lacks support) |
| `ModelRef` value at a `FROM`-clause or reducer splice site; field projection `m.path` / `m.name` | The model's source `.sql` file; graceful no-op when the concrete model cannot be determined without expansion context |
| `SourceRef` value at a `FROM`-clause or reducer splice site; field projection `s.path` / `s.name` | The source `.yml` file; graceful no-op when the concrete source cannot be determined without expansion context |
| `smelt.<path>` reference to a generator-emitted model (a model whose path was produced by a `generates: models` generator file) | The generator file's emitting `ModelDef` literal — specifically, the `ModelDef.name` field's value-expression token whose evaluation produced the emitted name |

Go-to-Definition on a `smelt.<path>` reference in a SQL model navigates to the file at that path. For Python-derived models, it navigates to the `.py` file at the line of the `@model` decorator for that function.

Column definition tracing follows `SELECT *` chains across multiple upstream models.

### Find References

Find References resolves the following identifier types:

| Identifier | Returns |
|------------|---------|
| `smelt.<path>` (at definition or use, resolves to a model / source / seed) | All `smelt.<path>` references to the same entity across the workspace |
| `smelt.<path>(...)` (at a call site, resolves to a function) | All `smelt.<path>(...)` call sites for the same function across the workspace, scoped to the function's project |
| `smelt.define <name>` declaration name token | All `smelt.<path>(...)` call sites resolving to that function, in the same project |
| CTE name (at definition or use) | All references to that CTE within the same file |

Cross-file find-references for `smelt.<path>` searches all loaded workspace files **within the same project** — a workspace folder may contain multiple smelt projects, and references do not cross project boundaries (see `architecture.md` → "Project isolation rule"). For CTEs, the search is intra-file by construction.

### Hover

Hover is supported on:

| Target | Hover content |
|--------|--------------|
| `smelt.<path>` (model) | Model schema as markdown table (columns, types, nullability) plus upstream lineage |
| `smelt.<path>` (source) | Source table schema from the source `.yml` |
| `smelt.<path>` (seed) | Seed schema (sidecar or inferred) plus row count |
| `smelt.columns_of(t)` call path | `List<ColumnRef>` plus, when `t`'s schema is statically resolvable, the resolved column count and the first five column names |
| `ColumnRef`-typed lambda parameter | `ColumnRef` plus the closed field list with each field's type (`name: Text`, `type: DataType`, `is_numeric: Boolean`) |
| Field projection `c.name` / `c.type` / `c.is_numeric` | The field's declared type |
| Meta-`Text` lifted as identifier | The lift description (`Text → identifier`) and, when statically traceable, the resolved column name |
| `smelt.models.with_tag(t)` call path | `List<ModelRef>` plus, when `t` resolves to a string literal, the match count and the first five matching model names |
| `smelt.models.all` call path | `() -> List<ModelRef>` plus the total model count for the project containing the current file |
| `smelt.sources.with_tag(t)` call path | `List<SourceRef>` plus, when `t` resolves to a string literal, the match count and the first five matching source names |
| `smelt.sources.all` call path | `() -> List<SourceRef>` plus the total source count for the project containing the current file |
| `ModelRef`-typed lambda parameter (bound by a HOF over a `smelt.models.*` list) | `ModelRef` plus the closed field list with each field's type (`path: Text`, `name: Text`, `tags: List<Text>`, `columns: List<ColumnRef>`) |
| `SourceRef`-typed lambda parameter (bound by a HOF over a `smelt.sources.*` list) | `SourceRef` plus the closed field list with each field's type (`path: Text`, `name: Text`, `tags: List<Text>`, `columns: List<ColumnRef>`) |
| Field projection `m.path` / `m.name` / `m.tags` / `m.columns` (on `ModelRef`) | The field's declared type |
| Field projection `s.path` / `s.name` / `s.tags` / `s.columns` (on `SourceRef`) | The field's declared type |
| `generates: models` frontmatter key or value | The inferred body type (`List<ModelDef>`) and, when statically resolvable, the count of emitted models |
| `ModelDef { … }` record literal (opening brace or `ModelDef` keyword) | The inferred emitted-model smelt path when the `name` field's value is statically known; otherwise `ModelDef` |
| `ModelDef.name` field-value expression | The resulting emitted smelt path derived from the field's value |
| `ModelDef.body` field-value expression | The body's synthesised `TableExpr` type and the inferred column list when resolvable |
| Any other `ModelDef` field-value token (`materialization`, `tags`, `description`) | The field's declared type from `MODEL_DEF_FIELDS` |

Hover content includes type annotations and row requirements from the type inference system where available.

### Completions

Completions are triggered by the characters `'`, `(`, and `.`.

| Context | Completions offered |
|---------|---------------------|
| After `smelt.` | All addressable entities in the project containing the current file (models, seeds, sources, functions), grouped by kind in the completion list |
| After a `smelt.<partial>` segment | Entities whose path begins with the entered segments |
| Column context (unqualified) | All reachable column names with inferred types |
| After `<alias>.` | Columns from the table/model/CTE bound to that alias |
| At `c.<cursor>` where `c: ColumnRef` | The closed field set (`name`, `type`, `is_numeric`) and no other identifiers |
| At `smelt.columns_of(<cursor>)` argument position | In-scope `TableExpr`-valued names (`smelt.<path>` model references and enclosing function `TableExpr` parameters) |
| At `smelt.models.<cursor>` | The closed accessor set (`with_tag`, `all`) and no other identifiers |
| At `smelt.sources.<cursor>` | The closed accessor set (`with_tag`, `all`) and no other identifiers |
| At `m.<cursor>` where `m: ModelRef` (lambda parameter over `smelt.models.*` list) | The closed field set (`path`, `name`, `tags`, `columns`) and no other identifiers |
| At `s.<cursor>` where `s: SourceRef` (lambda parameter over `smelt.sources.*` list) | The closed field set (`path`, `name`, `tags`, `columns`) and no other identifiers |

Schema-aware column completions derive types from the Salsa type inference system.

### Rename

Rename is supported on:

| Identifier | What is renamed |
|------------|----------------|
| CTE name | CTE definition and all references within the same file |
| `smelt.<path>` | All `smelt.<path>` references to that entity across the workspace; the source file itself is **not** renamed |
| Column name (model/CTE column) | The column at its **resolved definition site** and every transitive consumer, rewritten from that root: local file, downstream model files reachable through the dependency graph, and upstream model files traced to the definition. Propagation **terminates at an `AS` re-aliasing** (a consumer that renames the column under a new alias is the boundary — its alias and downstream uses are not rewritten). `SELECT *` chains **propagate** (the column flows through unnamed, so its rename carries downstream). |
| Lambda parameter | The parameter binder and every reference to it inside the lambda's body. Scope is the single lambda; inner lambdas that shadow the parameter are not touched. The new name must be a valid SQL identifier, must not collide with a meta-namespace keyword (`if`, `then`, `else`, `fn`, `let`), and must not shadow an outer binder already referenced inside the lambda body |

**Source columns are not renameable.** A column declared by a source `.yml` names a column of an externally-managed table that smelt does not own. Renaming it would rewrite the *declaration* — turning the source `.yml` green — while every runtime query against the real external table still references the old name and breaks. `prepare_rename` on a source column therefore responds **not-supported**, with an explanatory message that the table is external and its columns must be renamed at the source. Renaming a model/CTE column that *reads from* a source column rewrites the model-side references and stops at the source boundary; it never edits the source `.yml`.

`prepare_rename` is supported for the renameable identifiers above — editors can preview the rename range before committing. For a source column it returns not-supported as described.

The new name must be a valid SQL identifier. Invalid identifiers are rejected with an error.

Renaming a model name does not rename the SQL file on disk. The model name is derived from the file stem; to fully rename a model, the file must be renamed separately (which changes the model name automatically).

### Code Actions

| Action | Trigger | What it does |
|--------|---------|--------------|
| Create model from ref | Cursor on an `UndefinedModelRef` diagnostic | Generates a SQL file skeleton at the resolved path |
| Fix undefined ref | Cursor on parse/ref error | Offers text edit to correct the reference |
| Add column to source YAML | Cursor on undeclared source column | Inserts column declaration into the resolved per-entity source `.yml` |
| Extract CTE | Cursor on subquery expression | Extracts subquery into a named CTE in the WITH clause |
| Inline CTE | Cursor on CTE reference | Inlines the CTE body at the reference site |

## Design

**Salsa-based incremental updates.** All analysis — type inference, diagnostics, schema resolution — is implemented as Salsa queries. File changes set a new input value; only queries that transitively depend on the changed file are re-run. This makes re-analysis after a single file change fast regardless of workspace size.

**Pure functions, thin Salsa wrappers.** Analysis logic (type inference, diagnostics, schema extraction) is implemented as pure functions. Salsa queries are thin wrappers that supply inputs and cache results. This architectural invariant allows the same logic to be used by the LSP (via Salsa) and by the CLI planner (directly, without Salsa).

**Full text sync.** The server uses full text synchronization rather than incremental text. This is simpler and sufficient given that Salsa handles incremental analysis. The text diff is not needed at the LSP level.

**Cross-file rename via graph traversal, rooted at the definition site.** Column renames resolve the column's definition site first, then rewrite every transitive consumer from that root (downstream through the dependency graph; upstream traced back to the definition). Rooting at the definition rather than the invocation file makes the rewritten set independent of where the rename was triggered. Re-aliasing (`AS`) is a deliberate propagation boundary, and source columns are refused outright (renaming an external table's declaration cannot be made safe). This ensures that renaming a column in a base model also updates derived models that expose or transform that column, without silently breaking external-table contracts.

**Python models as virtual SQL files.** Python-generated models are registered as virtual `.sql` paths in the Salsa database. Editor positions in Python files are mapped to virtual file boundaries; diagnostics and go-to-definition work through this virtual path layer.

## Constraints & Invariants

1. **Diagnostics are always current with the latest saved file text.** Diagnostics may be stale between a file change and the server's next analysis cycle, but they are never based on text older than the most recent `textDocument/didChange` notification processed.
2. **Rename does not rename SQL files on disk.** Renaming a model via LSP rename only updates references. The file rename must be done separately.
3. **Go-to-definition on ambiguous unqualified columns returns multiple locations.** This is the LSP `GotoDefinitionResponse::Array` form; editors render it as a picker.
4. **Column completion includes types.** Completion items for columns include the inferred type as a detail label. If the type is unknown, the label is omitted.
5. **Code action "Create model" generates a SQL skeleton.** The generated file is a minimal valid smelt SQL model; it does not include frontmatter beyond the defaults.
6. **Workspace loading is shared with the CLI.** `Backend::initialize` consumes `smelt_core::workspace::load_workspace` and `smelt_db::workspace_ingest::ingest_loaded_workspace` — see `docs/specs/architecture.md` → "Workspace loading parity rule (CLI ↔ LSP)". The standing safety net is `cargo test -p smelt-lsp --test example_workspaces`, which drives the real `Backend` against every non-broken example workspace and asserts no diagnostics.
7. **A VSCode workspace folder may contain multiple smelt projects.** `find_smelt_projects` discovers them recursively; each project is a closed resolution scope (no cross-project `smelt.<path>` resolution). See `docs/specs/architecture.md` → "Project isolation rule".

## Known Divergences / Open Questions

- **Rename does not rename the file.** Renaming a model via LSP updates all references but not the source file. This is a known gap — after a rename, the model name reverts to the file stem on next discovery unless the file is also renamed.
- **Record-field rename is not supported.** Record types are structural and anonymous; renaming a field would have to propagate through every record-literal constructor, every projection, and every loader `schema` argument that uses the type. Prepare-rename on a record field name responds with not-supported. Tracked as a v2 enhancement.
- **Python LSP support is partial.** Go-to-definition from SQL to Python `@model` functions works. Diagnostics for type errors *inside* the Python-generated SQL are attributed to the virtual SQL location, not the Python source line.
- **Source `.yml` files are not watched.** Changes to a per-entity source YAML require reopening the workspace or making a model file change to trigger re-analysis. The server does not watch source `.yml` files independently.
- **`smelt.yml` changes require server restart.** Project configuration changes (new model paths, target changes) are not detected dynamically; the LSP server must be restarted.
- **Hover on CTEs not implemented.** Hover resolves `smelt.<path>` references but not CTE names or column references.
- **Find-references for columns not implemented.** Find References resolves model/source/seed paths, function paths, and CTEs, but not column names. Column rename works (it walks the dependency graph), but surfacing all uses of a column without renaming is not supported.
- **Find-references gaps for other identifier kinds.** The following kinds support go-to-definition but not find-references and are tracked for future work: table aliases (intra-file), lambda parameters (intra-lambda), Python `@model` functions (would return SQL `smelt.<path>` call sites), and `smelt.columns_of` / `smelt.models.*` / `smelt.sources.*` accessor call paths.
- **Unresolved-reference codes.** Unresolved `smelt.<path>` value references use the two catalogued codes `UndefinedModelRef` (the default for a bare unresolved path) and `UndefinedSource` (a reference resolving to a missing/invalid source); unresolved call forms use `UnknownSmeltFn` (`functions.md`). There is no `UnknownSmeltPath` code. The kind-aware *message* is still produced from the use-site's expected kind, but the *code* follows the resolver's classification.
- **Diagnostic code ownership.** The full `DiagnosticCode` catalogue — every variant, its severity, and its trigger — is maintained in `docs/specs/diagnostics.md`. The LSP surfaces every catalogued code; ownership and stability tiers are documented there.

## References

- **Code**:
  - `crates/smelt-lsp/src/lib.rs` — main LSP server, all capability handlers
  - `crates/smelt-lsp/src/python_scan.rs` — Python decorator scanning for go-to-definition
  - `crates/smelt-db/src/lib.rs` — Salsa queries, `file_diagnostics()`, `model_schema()`
  - `crates/smelt-db/src/type_inference.rs` — pure type inference functions
  - `crates/smelt-db/src/code_actions.rs` — code action suggestions
  - `crates/smelt-db/src/references.rs` — `find_cte_references()`
- **User docs**:
  - `docs-site/docs/guide/editor-features.md`
  - `docs-site/docs/guide/editor-setup.md`
- **Related specs**:
  - `models.md` — `smelt.<path>` addressing, frontmatter schema
  - `sources.md` — per-entity source `.yml` format, `smelt.<path>` resolution to sources, ownership of `MalformedSource` / `SourceTypeError` codes
  - `functions.md` — `smelt.define`, `smelt.extern`, function-related diagnostic codes
  - `types.md` — type inference system, DataType vocabulary
