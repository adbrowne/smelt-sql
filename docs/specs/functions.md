---
feature: functions
status: experimental
last_reviewed: 2026-04-29
owners: [andrew]
---

# Functions

> **Scope.** Normative spec for the user-facing function surface: `smelt.define`, `smelt.fn.*` calls, `smelt.extern`, `PASSING` clauses, `smelt.as_struct`, function frontmatter, default values, and the cycle/overload/recursion rules. Type vocabulary and fragment-sort rules live in `types.md` and are referenced — not duplicated — here. Scoping inside bodies (parameters-first, no-overlap, splice-context inference) lives in `scoping.md`. The three-tier checking model lives in `gradual_typing.md`. Planner integration of frontmatter properties lives in `planner_integration.md`.

## Surface

### File structure

A `.sql` file is a sequence of top-level **items**. Each item is one of:

- A `smelt.define` declaration.
- A `smelt.extern` declaration.
- A bare model `SELECT` (the file's model body).

Each item may be preceded by an optional YAML **frontmatter** block (`---` … `---`). Frontmatter attaches to the immediately following declaration; there is no file-level frontmatter scope. (Research §16 #22.)

Rules:

- Items are separated by whitespace only — no separator token.
- A file may contain **at most one** bare model `SELECT`. Multiple model `SELECT`s in one file are not in the grammar (the second one is a parse error).
- A file may contain **zero or more** `smelt.define` and `smelt.extern` items, interleaved freely with each other and with the optional model `SELECT`.
- File **kind** (model file vs. function file) is a function of grammar, not directory placement (architecture spec, "Project layout"). The directory only drives `smelt.fn.*` namespacing — `functions/patterns/session_rollup.sql` declares `smelt.fn.patterns.session_rollup`.
- A trailing `;` after a `smelt.define` or `smelt.extern` declaration is allowed but optional.

### `smelt.define` grammar

```
smelt.define <name>(<param-list>) [-> <Type>] AS (<body>) [;]
```

- **Name.** Bare identifier. Function paths under `smelt.fn.*` are derived from the directory layout under `functions/` plus the declared name.
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

### `smelt.fn.*` call syntax

```
smelt.fn.<path>(<arg-list>)
```

- `<path>` is the directory-derived dotted namespace plus the function name.
- Arguments may be positional or named with `param => value` (PostgreSQL/Oracle convention).
- Named-argument syntax does **not** apply to variadic positions.
- Externs are called by their **bare** declared name (e.g. `read_parquet(x)`), not via `smelt.fn.*`. Built-ins are likewise called by bare name.

### `PASSING` clauses

```
smelt.fn.foo(<inline-args>)
PASSING <name1> AS (<body1>)
PASSING <name2> AS (<body2>)
…
```

- `PASSING` is a **context-sensitive keyword** (research §16 #18). It is reserved only at the syntactic position immediately following the closing `)` of a smelt function call (`smelt.fn.<…>(...)` or a call to a `smelt.define`-declared function). Everywhere else (column names, aliases, CTE names, ordinary identifiers) `PASSING` is a regular identifier.
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

Model frontmatter keys (e.g. `materialization`, `incremental`) are catalogued in `incremental_models.md` and the architecture spec — not duplicated here. The frontmatter parser is shared across all three declaration kinds.

### Diagnostic codes

User-visible codes anchored to the surface above. Full descriptions live alongside `DiagnosticCode` in `crates/smelt-db/src/lib.rs`.

| Code | Triggered by |
|---|---|
| `DuplicateFunctionDefinition` | Two `smelt.define`s (or `smelt.extern`s) share a name in the workspace. |
| `DuplicateParameterName` | Two parameters in one signature share a name. |
| `UnknownSmeltFn` | `smelt.fn.<path>(...)` references an unregistered function. |
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

1. **All functions are public.** `smelt.define` has no visibility modifier in v1. The function's directory-derived namespace path is its identity. Adding visibility is non-breaking (default stays public).
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
11. **Multiple defines per file** (research §16 #6). A file is a compilation unit, not a one-definition container. Defines and the optional bare model `SELECT` may interleave freely.
12. **Frontmatter attachment.** Each frontmatter block attaches to the immediately following declaration. Each declaration may carry its own. There is no file-level frontmatter and no frontmatter inheritance across declarations.
13. **`PASSING` parses without type information.** The trigger rule (one-token lookahead after `)`) does not require knowing the callee's parameter list. Name validation, sort compatibility, and binding all run after parsing in the type-checker.
14. **Externs treated as atomic.** `smelt.extern` calls are checked against their declared signature exactly like built-ins. The planner treats them as atomic nodes (see `planner_integration.md`).
15. **Error recovery.** `smelt.define`, `smelt.extern`, and the frontmatter fence `---` are all safe resync tokens. Unrecoverable errors inside a declaration skip tokens until the next top-level boundary (`smelt.define`, `smelt.extern`, `---`, or EOF). Errors inside a body's `(...)` use standard Rowan SQL error recovery.

### Interactions with adjacent specs

- **Type vocabulary, fragment sorts, generics inference, variadics, bidirectional checking** — see `types.md`. This spec assumes those rules; it does not restate them.
- **Body scoping (parameters-first, no-overlap, splice-context inference, `Expr<T, ctx>` semantics)** — see `scoping.md`.
- **Three-tier checking (Tier 1 / 2 / 3 dispatch, error-tracing contract, LSP stability)** — see `gradual_typing.md`.
- **Planner consumption of `deterministic` / `idempotent` / `append_only` / `joins` / `provenance`** — see `planner_integration.md`.
- **Models-as-functions equivalence (a model is a `smelt.define` whose `TableExpr` parameters default to refs/sources)** — see `architecture.md`.

## Constraints & Invariants

1. The `smelt.define` body is parenthesised. The closing `)` of the body terminates the declaration without lookahead into SQL.
2. The transparent-function call graph is acyclic. The cycle pre-pass in `smelt-db` runs workspace-wide and feeds the planner; downstream stages may assume termination.
3. `smelt.fn.*` paths are stable identifiers — they are derived from directory layout plus the declared name and do not depend on file contents elsewhere in the workspace.
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
- **End-to-end `smelt build` execution of `smelt.fn.*` calls is incomplete.** Phases 56–57 of `docs/plans/20260422-smelt-functions.md` cover the codegen integration that finalises function expansion at build time. LSP-time checking and `--show-plan` work today; the full build-and-execute path is in progress.
- **Frontmatter validation depth.** Unknown keys currently emit `FrontmatterParseError` at Warning severity, which means typos like `deterministc: true` are silently ignored beyond a warning. Whether this should escalate to Error is open.
- **Workspace-wide vs. directory-scoped name uniqueness.** The implementation today applies `DuplicateFunctionDefinition` workspace-wide. The research originally framed it directory-scoped; the workspace rule is stricter and matches the single canonical-namespace doctrine, but the spec author should confirm this is intended before treating it as final.

## References

### Code

- `crates/smelt-parser/src/parser.rs` — `parse_smelt_define`, `parse_smelt_extern`, `parse_smelt_fn_call`, `parse_passing_clause`, `parse_smelt_as_struct`, `at_smelt_*_trigger`
- `crates/smelt-parser/src/syntax_kind.rs` — `SMELT_DEFINE`, `SMELT_EXTERN`, `SMELT_FN_CALL`, `CALL_PATH`, `PASSING_CLAUSE`, `PASSING_NAME`, `PASSING_BODY`, `SMELT_AS_STRUCT_CALL`, `EXCEPT_COL_LIST`
- `crates/smelt-parser/src/ast.rs` — `SmeltDefine`, `SmeltExtern`, `SmeltFnCall` AST wrappers
- `crates/smelt-types/src/signatures.rs` — `FunctionSig`, `Tier`, `ParamSpec`, `BackendSet`, `extract_signature`, `extract_extern_signature`, `extract_function_signatures`, `parse_frontmatter_backends`
- `crates/smelt-db/src/lib.rs::DiagnosticCode` — every diagnostic code listed in Surface
- `crates/smelt-db/src/function_body_check.rs` — body checking (`check_function_body`, `check_smelt_fn_call`), Tier dispatch (`is_tier2_function`, `check_tier3_return_type`), `PASSING` validation, frame-stack construction
- `crates/smelt-db/src/functions.rs` — function registry / lookup
- `crates/smelt-db/src/backends.rs` — `infer_body_backends`, `apply_narrow_rule`, `resolve_backends`
- `crates/smelt-db/src/provenance_validator.rs` — `ProvenanceMismatch` / `JoinsMismatch` checks (Phase 51)

### Tests

- `crates/smelt-db/src/function_body_check.rs::tests` — body-check unit tests
- `crates/smelt-db/tests/` — workspace-level function tests (duplicate detection, cycle detection, PASSING)
- `examples/test_workspace/functions/` — worked examples that the LSP-diagnostics integration test runs against

### User docs

- `docs-site/docs/concepts/functions.md` (and adjacent `smelt.fn.*` / `smelt.extern` pages) — to be reconciled against this spec via `/smelt:validate functions`

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — primary implementation plan; Phases 1–57 cover the surface in this spec
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/architecture.md` — system pipeline, project layout, models-as-functions
- `docs/specs/types.md` — type vocabulary, fragment sorts, generics inference, bidirectional checking, variadics
- `docs/specs/scoping.md` — body-scope name resolution (parameters-first, no-overlap, splice contexts)
- `docs/specs/gradual_typing.md` — Tier 1/2/3 checking model and error-tracing contract
- `docs/specs/planner_integration.md` — how frontmatter properties feed planner rules
- `docs/specs/incremental_models.md` — model-frontmatter keys (`materialization`, `incremental`)

### Research

- `docs/research/20260413-smelt-functions.md` — sections 3, 4, 5, 10, 13, 16 (decisions 6, 11, 18, 19, 20, 21, 22, 23) are the source for this spec
