---
feature: functions
status: experimental
last_reviewed: 2026-05-04
owners: [andrew]
---

# Functions

> **Scope.** Normative spec for the user-facing function surface: `smelt.define`, `smelt.<path>(...)` calls, `smelt.extern`, `PASSING` clauses, `smelt.as_struct`, function frontmatter, default values, and the cycle/overload/recursion rules. Type vocabulary and fragment-sort rules live in `types.md` and are referenced — not duplicated — here. Scoping inside bodies (parameters-first, no-overlap, splice-context inference) lives in `scoping.md`. The three-tier checking model lives in `gradual_typing.md`. Planner integration of frontmatter properties lives in `planner_integration.md`. The universal `smelt.<path>` addressing scheme (which produces the function's call path from its file location plus declared name) is specified in `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme".

## Surface

### File structure

A `.sql` file is a sequence of top-level **items**. Each item is one of:

- A `smelt.define` declaration.
- A `smelt.extern` declaration.
- A bare model `SELECT` (a SELECT carrying `materialization: test` is a test model — declaration shape and assertion semantics owned by `testing.md`).

Each item may be preceded by an optional YAML **frontmatter** block (`---` … `---`). Frontmatter attaches to the immediately following declaration; there is no file-level frontmatter scope. (Research §16 #22.)

Rules:

- Items are separated by whitespace only — no separator token.
- A file may contain **any number** of bare model `SELECT`s (test or otherwise). The naming rule (lone-anonymous OR all-named via frontmatter `name:`, never mixed) is specified in `architecture.md` §"Project layout — Bare-model naming".
- A file may contain **zero or more** `smelt.define` and `smelt.extern` items, interleaved freely with each other and with bare model `SELECT`s.
- All declared names within a file (bare-SELECT names, `smelt.define`s, `smelt.extern`s) must be unique.
- File **kind** is a property of each declaration, not of the file (architecture spec, "Resolution"). The directory containing the file contributes to the entity's `smelt.<path>` namespace — e.g. `functions/patterns/session_rollup.sql` declaring `session_rollup` produces the call path `smelt.functions.patterns.session_rollup`. Externs are flat and ambient: their declaring path affects navigation only, never the call surface (see `architecture.md` §"Externs are flat").
- A trailing `;` after a `smelt.define` or `smelt.extern` declaration is allowed but optional.

### `smelt.define` grammar

```
smelt.define <name>(<param-list>) [-> <Type>] AS (<body>) [;]
```

- **Name.** Bare identifier. The function's call path under `smelt.<path>` is derived from the workspace-relative directory of the declaring file plus the declared name (architecture spec §"Resolution"). Renaming a function or moving its file changes the call path, exactly like renaming or moving a model.
- **Parameter list.** Balanced `(...)`. Each parameter is `name [: <Type>] [= <default>]`. Trailing commas are allowed.
- **Return arrow.** Optional `-> <Type>`. Presence of the arrow controls Tier 3 dispatch (see `gradual_typing.md`).
- **Body marker.** Required `AS` keyword (case-insensitive).
- **Body.** Balanced `(<expr-or-select>)`. The outer parens are required and make termination unambiguous without lookahead into SQL.

`<Type>` for parameters and returns uses the fragment-sort vocabulary specified in `types.md` (§"`smelt.define` type annotations"): `Expr<T[, ctx]>`, `AggExpr<T>`, `WindowExpr<T>`, `TableExpr` / `TableExpr<{…}>`, `SelectItems<Kind[, ctx]>`, `OrderSpec[<ctx>]`, open structs `Expr<Struct<{…, ..r}>>`. Generics (`<T: Constraint>`) and trailing variadic `...` are permitted only in built-ins and `smelt.extern` — `smelt.define` is monomorphic and non-variadic in v1.

### `smelt.extern` grammar

```
smelt.extern <name>(<param-list>) -> <Type> [;]
```

- No `AS (…)` body — externs are signature-only declarations.
- Return type is **mandatory** (externs are always Tier 3).
- Parameter list shape is identical to `smelt.define`.
- Backend-namespace sugar: `smelt.extern duckdb.read_parquet(...)` is equivalent to declaring `smelt.extern read_parquet(...)` with frontmatter `backends: { duckdb: { emit: read_parquet } }`.
- Externs share a name namespace with the canonical built-in registry.

### Function call syntax

```
smelt.<path>(<arg-list>)
```

- `<path>` is the workspace-relative directory of the declaring file (segments separated by `.`) joined with the function name. The filename stem is **not** a path component. Examples: a `smelt.define session_rollup(...)` declared in `functions/patterns/x.sql` is called as `smelt.functions.patterns.session_rollup(...)`; a `smelt.define helper(...)` declared in `random/x.sql` is called as `smelt.random.helper(...)`. The same `smelt.<path>` resolution rule that locates models, seeds, and sources locates functions — see `architecture.md` §"Resolution".
- Arguments may be positional or named with `param => value` (PostgreSQL/Oracle convention).
- Named-argument syntax does **not** apply to variadic positions.
- Externs are called by their **bare** declared name (e.g. `read_parquet(x)`), not via `smelt.<path>`. Built-ins are likewise called by bare name. The bare-name namespace is workspace-wide; the declaring path of an extern is irrelevant to the call surface (see `architecture.md` §"Externs are flat").

#### Boolean-position placement

A `smelt.<path>(...)` call whose return type is `Expr<Boolean>` is valid in any boolean position the SQL grammar accepts: `WHERE`, `HAVING`, `JOIN ON`, `QUALIFY`, `CASE WHEN`, and as a `SELECT`-list expression. Splice-context kind ceilings (Semantics §7 in `types.md`) still apply — a function whose body is `Agg`-kinded is rejected in `WHERE`/`ON`/`GROUP BY` even if the declared return type is `Expr<Boolean>`. Example:

```sql
-- functions/orders.sql declares is_shipped(status TEXT) -> Expr<Boolean>
SELECT *
FROM smelt.orders
WHERE smelt.functions.is_shipped(status)
```

File-location → call-path mapping (path-prefix enforcement is normative; a wrong-prefix call emits `UnknownSmeltFn`):

| Filesystem location | Declared name | Call path |
|---|---|---|
| `functions/status.sql` | `is_shipped` | `smelt.functions.is_shipped(...)` |
| `functions/patterns/x.sql` | `session_rollup` | `smelt.functions.patterns.session_rollup(...)` |
| `utils/math.sql` | `safe_divide` | `smelt.utils.safe_divide(...)` |

### `PASSING` clauses

```
smelt.<path>(<inline-args>)
PASSING <name1> AS (<body1>)
PASSING <name2> AS (<body2>)
…
```

- `PASSING` is a **context-sensitive keyword** (research §16 #18). It is reserved only at the syntactic position immediately following the closing `)` of a smelt function call (any `smelt.<path>(...)` call, or a call to a `smelt.define`-declared function — equivalently, the same thing). Everywhere else (column names, aliases, CTE names, ordinary identifiers) `PASSING` is a regular identifier.
- Each clause binds a single fragment-typed parameter by name. Multiple clauses may attach to one call.
- Trigger rule: after the call's closing `)`, the parser peeks one token; if it is `PASSING`, a clause sequence begins; otherwise normal SQL parsing resumes.
- The trigger rule is uniform in expression position and FROM position.
- `PASSING` does **not** attach to plain SQL function calls (`UPPER(...)`, `SUM(...)`), nor to `smelt.extern` calls in v1 — externs declare no fragment-sort parameters.

### `smelt.as_struct(...)`

```
smelt.as_struct(<table-alias> [EXCEPT <col1>, <col2>, …])
```

- Compile-time struct namespacing: produces a struct expression whose fields are the columns of `<table-alias>`, optionally excluding listed columns (typically join keys).
- Used inside a function body to expose multiple joined tables to caller-provided fragments without column-name collisions (Strategy 3 of the no-overlap rule; see `scoping.md`).
- Backend support is a capability gate — see `AsStructUnsupportedBackend` below. Compiles to the engine's native struct literal (`{f: v}` on DuckDB, `struct(v AS f)` on Spark).
- v1 status — see Known Divergences.

### Function-declaration frontmatter

YAML keys recognised on a frontmatter block preceding a `smelt.define` or `smelt.extern`:

| Key | Value shape | Default | Meaning |
|---|---|---|---|
| `deterministic` | `true` / `false` | `false` | Same input → same output. Planner-visible (see `planner_integration.md`). |
| `idempotent` | `true` / `false` | `false` | Retry-safe at execution. |
| `append_only` | `true` / `false` | `false` | Emits only new rows; never updates or deletes. |
| `backends` | `all` (string) or list of backend names (e.g. `[duckdb, postgres]`) | inferred from body | Backend compatibility set. May only **narrow** what the body supports. |
| `joins` | structured map (shape TBD) | absent | Declared join graph for planner pushdown. Gated behind `smelt.yml: unstable_schema: true`. |
| `provenance` | structured map (shape TBD) | absent | Declared column-provenance map. Gated behind `smelt.yml: unstable_schema: true`. |
| `backends.<name>.emit` | string | declared name | (`smelt.extern` only) Backend-specific emitted name. |

Model frontmatter keys (e.g. `materialization`, `incremental`) are catalogued in `models.md` / `incremental_models.md` and the architecture spec — not duplicated here. The frontmatter parser is shared across all three declaration kinds (model `SELECT` — including `materialization: test` test models — `smelt.define`, and `smelt.extern`).

### Diagnostic codes

User-visible codes anchored to the surface above. Full descriptions live alongside `DiagnosticCode` in `crates/smelt-db/src/lib.rs`.

| Code | Triggered by |
|---|---|
| `DuplicateFunctionDefinition` | Two `smelt.define`s (or `smelt.extern`s) share a name in the workspace. |
| `DuplicateParameterName` | Two parameters in one signature share a name. |
| `UnknownSmeltFn` | A `smelt.<path>(...)` call references a path that does not resolve to a function (no file at that path, the file is not a `.sql`, the file does not declare a `smelt.define` of that name, or the path resolves to a non-callable kind such as a model or seed used in call position). |
| `MissingArgument` | Call omits a required (non-defaulted) parameter. |
| `ArgTypeMismatch` | Argument's type fails the parameter's `TypeConstraint`. |
| `FunctionBodyTypeMismatch` | Type error inside a `smelt.define` body. |
| `ReturnTypeMismatch` | Tier 3 body's synthesised return type disagrees with declared `-> <Type>`. |
| `InvalidFunctionTypeRef` | Type annotation does not parse into a `SmeltType`. |
| `FunctionCallCycle` | Transparent-function call graph contains a cycle. |
| `ExternCollidesWithBuiltin` | `smelt.extern` name shadows the canonical built-in registry. |
| `ExternFragmentParamUnsupported` | `smelt.extern` declares a fragment-sort parameter (`SelectItems`, `OrderSpec`). |
| `UnknownPassingParameter` | `PASSING <name> AS (...)` names a parameter not declared on the callee. |
| `BackendsWideningNotAllowed` | Declared `backends:` claims a backend the body does not support, or the frontmatter itself is malformed in this dimension. |
| `FrontmatterParseError` | YAML parse failure (Error) or unknown key / malformed sub-entry (Warning). |
| `UnstableSchemaRequired` | `provenance:` (or other gated key) used without `smelt.yml: unstable_schema: true`. |
| `AsStructUnsupportedBackend` | `smelt.as_struct(...)` called in a body whose declared backend set includes a backend without struct-literal support. |

## Semantics

These rules are normative.

1. **All functions are public.** `smelt.define` has no visibility modifier in v1. The function's `smelt.<path>` (workspace-relative directory plus declared name) is its identity. Adding visibility is non-breaking (default stays public).
2. **No overloading.** Function names are unique within their namespace. Overloading combined with gradual typing produces annotation-tier-dependent resolution rules and is excluded by construction.
3. **No recursion.** A function may not call itself, directly or transitively. The compiler runs a workspace-wide cycle pre-pass on the call graph and emits `FunctionCallCycle` at every declaration participating in a cycle. The planner aborts splicing for those `fn_id`s — generated SQL must never inline a non-terminating expansion.
4. **No nesting.** `smelt.define` and `smelt.extern` may not appear inside a SELECT, CTE, or another function body. They are top-level-only. Local/nested defines may be added later without breaking changes.
5. **One canonical built-in registry, not per-dialect** (research §13). Built-ins, `smelt.extern`s, and `smelt.define`s share a single name namespace. Backend availability is a per-function `backends:` property, not a registry split. Engine-native precision is opt-in via the backend namespace (`postgres.sum(...)`); the surface call site is a visible, typeable commitment to backend specificity.
6. **Canonical return types are CAST-enforced.** When a built-in's canonical return type differs from the engine's native return type (e.g. PostgreSQL's `SUM(integer) → numeric` vs. smelt's `SUM(Integer) → BigInt`), the generated SQL wraps the call in `CAST(... AS <canonical>)`. Calls made via the backend namespace (`postgres.sum(...)`) opt out of this CAST and inherit the engine's native type — and mark the model as non-portable.
7. **Backend-namespace calls are explicit.** A function body that calls `duckdb.read_parquet(...)` declares its DuckDB-only nature in the type system. Function bodies are otherwise written in canonical SQL — engine-agnostic by construction. The `backends:` set is inferred from the body as the intersection of the backends of every backend-namespace call (canonical calls contribute the universal set). A declared `backends:` may **narrow** that inferred set but never widen it; widening is an error (`BackendsWideningNotAllowed`).
8. **Parameter list constraints.**
   - Parameter names must be unique within a signature (`DuplicateParameterName`).
   - Type annotations must parse into a `SmeltType` (`InvalidFunctionTypeRef`).
   - `smelt.extern` parameters must be non-fragment sorts — `Expr<T>` and `TableExpr` are accepted, `SelectItems` and `OrderSpec` emit `ExternFragmentParamUnsupported`. Fragment-sort parameters are only meaningful with `PASSING` clauses, which `smelt.extern` does not support in v1.
9. **Default values are self-contained** (research §16 #20). A default expression must not reference other parameters. Defaults are type-checked against the parameter's declared sort (and concrete type, in Tier 2/3) at definition time. Tier 1 functions have no declared parameter types, so the default's synthesised type becomes the parameter's type when the argument is omitted.
   - List-valued fragment sorts (`SelectItems`, `OrderSpec`) do not acquire an implicit empty default — an author who wants "splice nothing" writes `= ()` explicitly. Empty list-splice points elide adjacent commas at codegen (so `SELECT id, name, metrics` with `metrics = ()` becomes `SELECT id, name`).
   - For `Expr<Boolean>` filter parameters that should default to "no filter," the idiom is `= TRUE`.
   - Defaults on row-polymorphic parameters are not permitted in v1.
10. **`smelt.metric()` is out of scope** (research §16 #6). It is a semantic-layer concept with different design constraints; this spec does not address it.
11. **Multiple defines per file** (research §16 #6). A file is a compilation unit, not a one-definition container. Defines, externs, tests, and bare model `SELECT`s may interleave freely. Naming uniqueness within a file is enforced across all four kinds.
12. **Frontmatter attachment.** Each frontmatter block attaches to the immediately following declaration. Each declaration may carry its own. There is no file-level frontmatter and no frontmatter inheritance across declarations.
13. **`PASSING` parses without type information.** The trigger rule (one-token lookahead after `)`) does not require knowing the callee's parameter list. Name validation, sort compatibility, and binding all run after parsing in the type-checker.
14. **Externs treated as atomic.** `smelt.extern` calls are checked against their declared signature exactly like built-ins. The planner treats them as atomic nodes (see `planner_integration.md`).
15. **Error recovery.** `smelt.define`, `smelt.extern`, and the frontmatter fence `---` are all safe resync tokens. Unrecoverable errors inside a declaration skip tokens until the next top-level boundary (`smelt.define`, `smelt.extern`, `---`, or EOF). Errors inside a body's `(...)` use standard Rowan SQL error recovery.
16. **Declared return type is authoritative for call-site typing.** When the type checker encounters a `smelt.<path>(...)` call, it looks up the function's declared return type. A `-> <Type>` annotation yields a concrete call-expression type only when the annotation resolves to a specific concrete type (`Concrete(T)` in the type constraint system) or to `Numeric` (which widens to `Double`). Polymorphic constraints (`Ordered`, `Any`) and absent return types (Tier 1/2 functions) all produce `Unknown` at the call site. The schema of any model that projects such a call reflects this rule: a column whose source expression is a `smelt.<path>(...)` call inherits the resolved type or `Unknown`. Downstream aggregate functions (`SUM`, `AVG`, etc.) apply their standard return-type rules to the resolved type — for example, `SUM(Double) → Double`.

### Interactions with adjacent specs

- **Type vocabulary, fragment sorts, generics inference, variadics, bidirectional checking** — see `types.md`. This spec assumes those rules; it does not restate them.
- **Body scoping (parameters-first, no-overlap, splice-context inference, `Expr<T, ctx>` semantics)** — see `scoping.md`.
- **Three-tier checking (Tier 1 / 2 / 3 dispatch, error-tracing contract, LSP stability)** — see `gradual_typing.md`.
- **Planner consumption of `deterministic` / `idempotent` / `append_only` / `joins` / `provenance`** — see `planner_integration.md`.
- **Models-as-functions equivalence (a model is a `smelt.define` whose `TableExpr` parameters default to `smelt.<path>`-resolved upstream refs/sources)** — see `architecture.md`.

## Design

This section captures the load-bearing rationale behind the surface and semantics above. Where deeper justification exists, it lives in `docs/research/20260413-smelt-functions.md` and is cross-linked.

**No overloading, no recursion.** Both rules are about predictability under gradual typing (`gradual_typing.md`). Overloading combined with three annotation tiers means resolution depends on which tier each call site is in — small annotation edits silently reroute dispatch and break error-message provenance. Recursion combined with macro-style splicing has no fixed point — expansion would either infinite-loop or require an arbitrary depth limit. Both are excluded by construction so the type checker is single-pass and so cycle detection can be a workspace-wide pre-pass that the planner can rely on (`FunctionCallCycle`, see also `architecture.md`). Tier-aware dispatch and recursive splicing were considered and rejected; both are non-breaking to add later if a motivating use case appears (research §3, §16 #6).

**One canonical built-in registry, not per-dialect.** A per-dialect registry would force every model to bind to a backend at signature-resolution time — the very coupling smelt is trying to break. The canonical registry keeps call sites dialect-agnostic by default; backend specificity is an explicit opt-in via the backend namespace (`postgres.sum(...)`, `duckdb.read_parquet(...)`). This composes with the `backends:` frontmatter property — backend compatibility becomes a property of a function (inferred and narrowable), the same shape as `deterministic` and `idempotent`, queryable by the same planner machinery (research §13).

**CAST-enforced canonical return types.** Output schemas are an ETL contract; users downstream of a model rely on them. If `SUM(integer)` returned smelt's `BigInt` on DuckDB and PostgreSQL's native `Decimal(38,0)` on PostgreSQL, the same model would write different schemas to different warehouses. The generated SQL therefore wraps canonical built-ins in `CAST(... AS <canonical>)` whenever the engine's native return type diverges. The backend namespace is the explicit opt-out — `postgres.sum(col)` inherits the engine type and marks the model non-portable in the `backends:` set (research §13).

**Directory-derived `smelt.<path>` namespacing for functions.** A `smelt.define session_rollup` in `functions/patterns/session_rollup.sql` is callable as `smelt.functions.patterns.session_rollup(...)` because the `smelt.<path>` resolver applies uniformly to every project-defined entity (architecture spec §"Resolution"). There is no manifest file to maintain and no import statement to write — discovery is a directory walk. An explicit-import / manifest design was considered and rejected as ceremony for negligible benefit at smelt's scale; the cost (renames touch call sites) is identical either way (research §3). The earlier `smelt.fn.*` prefix was retired alongside `smelt.ref(...)` and `smelt.source(...)` when addressing was unified; the rationale lives in `architecture.md` Design §"Single addressing scheme".

**`backends:` may only narrow, never widen.** The body is the source of truth for what a function can run on — a body that calls `duckdb.read_parquet(...)` cannot execute on Spark, and no frontmatter declaration should be allowed to claim otherwise. The inference rule (intersection of backend-namespace calls in the body) is a hard ceiling; declared `backends:` may shrink the set further (e.g., to mark a function as restricted for portability reasons even though its body would run anywhere) but never expand it. `BackendsWideningNotAllowed` fires when a declaration tries to widen — better a clear diagnostic than a generated SQL fragment that silently fails on the engine the metadata claimed to support (research §16 #23).

**`PASSING` as a dedicated clause, not function-call argument syntax.** Multi-line SQL fragments inside an argument list are visually awful and create grammar problems — the parser would need to look for a fragment-list terminator inside arbitrary expression syntax. A trailing `PASSING <name> AS (<body>)` clause makes the binding visually explicit, naturally accommodates multiple fragments, and has a clean parse trigger (one-token lookahead after the call's closing `)`). The keyword is borrowed from SQL/XML's `XMLTABLE ... PASSING`, where it means the same thing. `PASSING` is context-sensitive (only reserved at the post-`)` position) so it does not steal an identifier name from existing SQL. Inline named-argument syntax remains available as a fallback for short fragments (research §10, §16 #18).

**`smelt.as_struct` is compile-time, not runtime.** Compile-time generation of struct expressions keeps row-shape inference local to the type checker and avoids a portability cliff on engines without struct-literal support. Lowering to engine-native `STRUCT(...)` everywhere was considered and rejected because `Expr<Struct<{…, ..r}>>` row polymorphism has to know the field set at expansion time anyway — a runtime mechanism would not solve the problem and would forbid backends without struct support entirely. Keeping it compile-time also means `smelt.as_struct` is interchangeable with the no-overlap rule's other strategies (CTE alias rename, typed `TableExpr<{…}>` parameter) instead of a separate dispatch path. The full design is deferred (see Known Divergences) and revisited alongside struct row polymorphism (research §16 #19).

**`joins:` and `provenance:` gated behind `unstable_schema`.** The structured-map shape is design-incomplete — automatic provenance derivation requires a full lineage analyser and the manual-declaration shape is still being prototyped against real planner rules (`planner_integration.md`). Locking in syntax now risks shipping something that has to break on first contact with real planner-rule pressure. Gating behind `smelt.yml: unstable_schema: true` lets eager users move while signalling clearly that the surface may shift; `UnstableSchemaRequired` makes the opt-in visible. Once the planner-integration spec stabilises the shape, the gate is dropped without breaking existing usage (research §12).

**All functions are public in v1.** Adding `pub` / `private` visibility costs grammar surface, scope-resolution complexity, and per-namespace privacy rules — all to constrain a workspace that today fits in one repo. No multi-package or cross-team boundary exists yet to justify the tax. Visibility is purely additive when added later (the default stays public), so deferring it costs nothing future-compatibility-wise (research §3).

**`smelt.metric()` is explicitly out of scope here.** Metrics are a semantic-layer concept whose lifecycle is planner-driven aggregation expansion, not signature-driven splicing. Folding them into this spec would conflate two evolution paths. Metrics live in their own future spec; this spec governs only the function-shaped surface (research §16 #6).

## Constraints & Invariants

1. The `smelt.define` body is parenthesised. The closing `)` of the body terminates the declaration without lookahead into SQL.
2. The transparent-function call graph is acyclic. The cycle pre-pass in `smelt-db` runs workspace-wide and feeds the planner; downstream stages may assume termination.
3. `smelt.<path>` function paths are stable identifiers — they are derived from the declaring file's workspace-relative directory plus the declared name, and do not depend on file contents elsewhere in the workspace.
4. `smelt.extern` and `smelt.define` share one workspace-wide name namespace with built-ins. A clash is a hard error at the second declaration (or at the `smelt.extern` if it shadows a built-in: `ExternCollidesWithBuiltin`).
5. The frontmatter parser is shared across model, `smelt.define`, and `smelt.extern` declarations. Property semantics differ; the parsing contract does not.
6. Adding a new frontmatter key is non-breaking; a previously-unknown key produces a `FrontmatterParseError` at Warning severity until it is recognised.
7. A declared `backends:` set may only narrow the inferred set. The inference rule "intersection of backend-namespace calls in the body, intersected with declared `backends:`" is monotone and decidable.
8. Out of scope for v1 (intent — preserved here so future plans honour it):
   - User-defined polymorphism in `smelt.define` (`<T: Constraint>` on user functions).
   - User-defined variadics (`smelt.define foo(x: Expr<T>...)`).
   - Recursion of any kind.
   - Visibility modifiers on `smelt.define`.
   - Local/nested defines.
   - `PASSING` clauses on `smelt.extern` calls.
   - Defaults referencing other parameters.

## Known Divergences / Open Questions

- **`smelt.as_struct` is partially landed.** The grammar parses, the diagnostic `AsStructUnsupportedBackend` is wired, but per research §16 #19 the full design is deferred to post-v1 alongside struct row polymorphism (`Expr<Struct<{…, ..r}>>`). Strategies 1 (CTE rename) and 2 (typed `TableExpr<{…}>` parameter) are the recommended v1 paths; `smelt.as_struct` should be treated as "design sketch, available but not finalised" until Step 8 of the smelt-functions plan revisits it.
- **`joins:` and `provenance:` parsing is partial.** The keys are recognised in frontmatter and gated by `unstable_schema: true`; structured-map shape and the `ProvenanceMismatch` / `JoinsMismatch` validation phase land in Phase 51 of the smelt-functions plan. Until that lands, declaring these properties is unstable in both surface and behaviour.
- **End-to-end `smelt build` execution of `smelt.<path>(...)` function calls is incomplete.** Phases 56–57 of `docs/plans/20260422-smelt-functions.md` cover the codegen integration that finalises function expansion at build time. LSP-time checking and `--show-plan` work today; the full build-and-execute path is in progress.
- **Frontmatter validation depth — divergent from doctrine.** Function frontmatter is user-authored, so under `architecture.md` §"Constraints & Invariants" §8 (the unknown-key doctrine) it should reject unknown keys with an error, like model frontmatter does. The current implementation emits `FrontmatterParseError` at Warning severity instead, which means typos like `deterministc: true` are silently accepted past the warning. Aligning with the doctrine (escalating to Error) is a straightforward future change once an audit confirms no in-the-wild function frontmatter relies on the lenient behaviour.
- **Workspace-wide vs. directory-scoped name uniqueness.** The implementation today applies `DuplicateFunctionDefinition` workspace-wide. The research originally framed it directory-scoped; the workspace rule is stricter and matches the single canonical-namespace doctrine, but the spec author should confirm this is intended before treating it as final.

## References

### Code

- `crates/smelt-parser/src/parser.rs` — `parse_smelt_define`, `parse_smelt_extern`, `parse_smelt_path_form`, `parse_passing_clause`, `parse_smelt_as_struct`, `at_smelt_*_trigger`
- `crates/smelt-parser/src/syntax_kind.rs` — `SMELT_DEFINE`, `SMELT_EXTERN`, `SMELT_PATH_CALL`, `CALL_PATH`, `PASSING_CLAUSE`, `PASSING_NAME`, `PASSING_BODY`, `SMELT_AS_STRUCT_CALL`, `EXCEPT_COL_LIST`
- `crates/smelt-parser/src/ast.rs` — `SmeltDefine`, `SmeltExtern`, `SmeltPathCall` AST wrappers
- `crates/smelt-types/src/signatures.rs` — `FunctionSig`, `Tier`, `ParamSpec`, `BackendSet`, `extract_signature`, `extract_extern_signature`, `extract_function_signatures`, `parse_frontmatter_backends`
- `crates/smelt-db/src/lib.rs::DiagnosticCode` — every diagnostic code listed in Surface
- `crates/smelt-db/src/function_body_check.rs` — body checking (`check_function_body`, `check_smelt_path_call`), Tier dispatch (`is_tier2_function`, `check_tier3_return_type`), `PASSING` validation, frame-stack construction
- `crates/smelt-db/src/functions.rs` — function registry / lookup
- `crates/smelt-db/src/backends.rs` — `infer_body_backends`, `apply_narrow_rule`, `resolve_backends`
- `crates/smelt-db/src/provenance_validator.rs` — `ProvenanceMismatch` / `JoinsMismatch` checks (Phase 51)

### Tests

- `crates/smelt-db/src/function_body_check.rs::tests` — body-check unit tests
- `crates/smelt-db/tests/` — workspace-level function tests (duplicate detection, cycle detection, PASSING)
- `examples/test_workspace/functions/` — worked examples that the LSP-diagnostics integration test runs against

### User docs

- `docs-site/docs/concepts/functions.md` (and adjacent `smelt.<path>` call / `smelt.extern` pages) — to be reconciled against this spec via `/smelt:validate functions`

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — primary implementation plan; Phases 1–57 cover the surface in this spec
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/architecture.md` — system pipeline, project layout, models-as-functions
- `docs/specs/types.md` — type vocabulary, fragment sorts, generics inference, bidirectional checking, variadics
- `docs/specs/scoping.md` — body-scope name resolution (parameters-first, no-overlap, splice contexts)
- `docs/specs/gradual_typing.md` — Tier 1/2/3 checking model and error-tracing contract
- `docs/specs/planner_integration.md` — how frontmatter properties feed planner rules
- `docs/specs/incremental_models.md` — model-frontmatter keys (`materialization`, `incremental`); see §"Functions inside incremental bodies" for how transparent and opaque calls interact with per-model WHERE injection and batch-safety classification

### Research

- `docs/research/20260413-smelt-functions.md` — sections 3, 4, 5, 10, 13, 16 (decisions 6, 11, 18, 19, 20, 21, 22, 23) are the source for this spec
