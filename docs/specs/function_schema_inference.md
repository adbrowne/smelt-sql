---
feature: function_schema_inference
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Function Schema Inference

> **What this is.** A normative spec for how a `smelt.functions.<name>(...)` call contributes **columns and their types** to the schema of the caller that selects from or projects it — the three return shapes (`Expr<T>`, `Struct<{…}>` spread via `.*`, `TableExpr`), how a `TableExpr` result's schema propagates through CTEs, derived-table subqueries, and JOIN aliases, and the contract that a caller column the inference cannot type is surfaced, never silently dropped. Out of scope: the `DataType` vocabulary and the `Unknown` reason-discriminant / no-silent-`Unknown` invariant (`types.md` §"Strict-by-default doctrine"); tier dispatch and call-site error tracing (`gradual_typing.md`); declaration grammar and frontmatter (`functions.md`); AST-level call expansion and SQL codegen (`expansion.md`); body-scope name resolution (`scoping.md`); meta-language reflection, generator-emitted model schemas, and HOF values in data position (`meta_language.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

A model body, a `smelt.define` body, and a function body are all `SELECT`-shaped and resolve column schemas by the same rules (`types.md` §"Models are functions"). This spec governs the columns a `smelt.functions.<name>(...)` call site contributes to whichever body contains it.

### Contributed columns by return shape

| Call form | Return sort of `<name>` | Columns contributed to the caller |
|---|---|---|
| `<name>(args) AS c` (SELECT scalar) | `Expr<T>` / `AggExpr<T>` / `WindowExpr<T>` | one column `c` of type `T` |
| `<name>(args).*` (SELECT) | `Expr<Struct<{f1: T1, …, fN: TN}>>` (closed struct) | `N` columns `f1…fN`, in declared field order, with their declared types |
| `FROM <name>(args) [AS a]` | `TableExpr` / `TableExpr<{…}>` | the function's resolved output schema, as the schema of alias `a` (or unqualified FROM scope) |

The contributed columns are observable via `smelt type` (the `inputs -> outputs` signature) and LSP hover, and they flow into every downstream consumer of the model's `ModelSchema` (further inference, `schema_evolution.md` diffs, `data_catalog.md`).

### Diagnostic codes (user-visible)

- `ColumnTypeUnresolved` — fires at the projection (the SELECT item or FROM entry) whose contributed column type the rules below cannot resolve, when the cause is a compiler-resolvable gap rather than a genuinely dynamic value. The column's type is `Unknown` with reason `Unresolved` (`types.md`). Message names the column and the unresolved source, e.g. *"cannot resolve the type of column `margin` produced by `smelt.functions.add_margin(...)`"*. It fires by default — there is no opt-in strictness flag (`types.md` §"Strict-by-default doctrine").

This spec emits no other codes of its own; argument-shape and row-requirement errors are owned by `gradual_typing.md` / `types.md` (`ArgTypeMismatch`, `MissingArgument`, `RowRequirementUnsatisfied`).

## Semantics

These rules are normative.

### 1. Scalar returns

A `smelt.functions.<name>(args)` call in scalar position contributes one column whose type is the call's resolved return type per `gradual_typing.md` tier dispatch: Tier 3 resolves from the declared `-> Expr<T>` signature; Tier 1 resolves by per-call-site expansion; `Expr<Numeric>` resolves to the LUB of the numeric arguments; `Expr<Any>` and unannotated returns resolve to `Unknown` with reason `Dynamic` (legitimately unknowable — not a `ColumnTypeUnresolved`). A declared concrete return (`-> Expr<Double>`) must resolve to that type and must not degrade to `Unknown`.

### 2. Struct returns and `.*` spread

A function returning a closed `Expr<Struct<{f1: T1, …, fN: TN}>>` (no row-tail marker) contributes columns via `<name>(args).*`: the spread expands to all `N` declared fields, in declared order, each typed by its declared field type. The expansion happens at the schema layer — the contributed `ModelSchema` columns are the struct fields — not only in generated SQL.

### 3. `TableExpr` returns in FROM

A `TableExpr`-returning function in a FROM or JOIN slot contributes its **output schema** as the schema of its alias. The output schema is the function body's resolved schema:

- A qualified wildcard `source.*` in the body expands against the schema of the `TableExpr` parameter `source`.
- A bare `*` in the body expands against the sole `TableExpr` parameter when there is exactly one; with multiple `TableExpr` parameters a bare `*` is `Unresolved`.
- An explicit body projection (with or without alias, including computed columns and window expressions) is typed by inferring its expression under the body's type context, which is seeded from each `TableExpr` argument's resolved schema and the workspace function signatures.

The `TableExpr` argument's schema is resolved from a `smelt.<path>` model/source reference, recursively from a nested `smelt.functions.<name>(...)` call in argument position, or from a local CTE or derived table named in the caller's `WITH` clause.

### 4. Propagation through CTEs, subqueries, and joins

A `TableExpr`-function output schema (rule 3) is a first-class schema: it propagates through the caller's body **structurally and transparently**. A model-level CTE, a derived-table subquery, or a JOIN alias whose FROM entry is a `smelt.functions.<name>(...)` call resolves the same columns as the direct FROM form. Selecting a column from such a CTE/subquery/alias yields the column's resolved type, not `Unknown`. There is no schema-resolution penalty for wrapping a function call in a CTE or subquery.

### 5. Resolution requires the callee in scope

Resolving the columns a `smelt.functions.<name>(...)` call contributes (rules 1–4) requires `<name>`'s signature to be discoverable in the project under analysis. Every entry point that resolves caller schemas — the LSP, the CLI `type` / `build` / `run` paths — must make function definitions discoverable through the shared workspace loader (`architecture.md` §"Workspace loading parity rule"). An entry point that resolves model schemas without function discovery resolves every `smelt.functions.*`-derived column to `Unknown`; this is a parity defect, not a property of the inference rules.

### 6. Unresolved columns are surfaced, never silent

A column contributed by rules 1–4 whose type the rules cannot resolve from a present, well-formed signature is `Unknown` with reason `Unresolved` and emits `ColumnTypeUnresolved` at the projection. A column that is `Unknown` only because an upstream contributed column was already `Unknown` carries reason `Propagated` and emits no further diagnostic (origin-only reporting — `types.md` §"Strict-by-default doctrine", `gradual_typing.md` single-primary-span rule). A column that is legitimately dynamic (`Expr<Any>`) carries reason `Dynamic` and is never a diagnostic.

## Design

**Struct fields expand at the schema layer, not only in codegen.** A model's output `ModelSchema` is a first-class artifact: downstream model inference, `schema_evolution.md` diffs, and `data_catalog.md` all read it. Expanding `f(...).*` only when generating backend SQL — leaving the schema layer to record zero columns for the spread — makes the schema lie about its own shape, and every downstream consumer inherits the lie as `Unknown`. Expanding at the schema layer keeps the inferred schema and the emitted SQL describing the same row.

**`TableExpr` schemas propagate structurally so CTEs and subqueries are transparent.** Once a `TableExpr`-function call resolves to a concrete column set, that set behaves exactly like a base table's schema for the rest of the body. The alternative — special-casing function calls only in the top-level FROM and treating a CTE/subquery wrapper as opaque — was rejected because authors routinely wrap a function call in a CTE to name it and aggregate over it (the canonical `WITH sessionized AS (SELECT … FROM smelt.functions.sessionize(...)) …` shape); making that wrapping erase types would punish idiomatic SQL and is indistinguishable to the user from a bug.

**Caller-schema resolution depends on function discovery, and that dependency is a parity invariant.** Because rules 1–4 consult the callee's signature (and, for `TableExpr`, re-walk its body), a consumer that loads models but not functions silently resolves every function-derived column to `Unknown` — passing both the zero-diagnostic gate and runtime-output snapshots while the inferred schema is wrong. Pinning the dependency to the shared workspace loader (`architecture.md`) makes the failure structurally impossible rather than relying on each consumer to remember function discovery.

**Unresolved columns must be loud; this follows existing doctrine rather than adding a flag.** `types.md` §"Strict-by-default doctrine" already requires every `Unknown` to carry a diagnostic and explicitly rejects configurable strictness. `ColumnTypeUnresolved` is the schema-layer instance of that rule: a function-derived column the compiler cannot type is a defect to report, not a value to tolerate. No opt-in `strict_types` flag is introduced, because an optional strict diagnostic gets negotiated away on large projects and the framework loses its strongest correctness lever. The reason-discriminant (`Unresolved` / `Dynamic` / `Propagated`, owned by `types.md`) is what lets the diagnostic fire exactly once at the origin without a cascade of duplicate errors at every downstream consumer.

## Constraints & Invariants

1. **Pure-function rule.** Schema inference (struct-field expansion, `TableExpr` body-schema resolution, propagation through CTEs/subqueries) is pure functions of AST nodes, `FunctionSig`, `TypeContext`, and resolved upstream schemas — no Salsa references in the analysis logic. Salsa queries are thin wrappers (CLAUDE.md "Pure Function Rule"; `architecture.md`).
2. **Schema-layer/codegen agreement.** The columns a call contributes to a model's resolved `ModelSchema` must equal the columns the emitted SQL for that call produces. Struct-spread and `TableExpr` expansion may not diverge between the two layers.
3. **Caller-schema resolution routes function discovery through the shared loader.** No entry point may resolve `smelt.functions.*`-derived schemas off a workspace that was populated without function definitions (`architecture.md` §"Workspace loading parity rule").
4. **No silent `Unknown`.** Every function-derived column that is `Unknown` with reason `Unresolved` is accompanied by a `ColumnTypeUnresolved` diagnostic at its origin. `Propagated` and `Dynamic` unknowns are diagnostic-free by construction (`types.md`).
5. **Out of scope for v1**: inferring schemas of generator-emitted models and reflected (`smelt.columns_of`) column sets (`meta_language.md`); typing meta-language HOF results placed in SQL column position (`meta_language.md`); multiple `TableExpr` parameters disambiguating a bare body `*`.

## Known Divergences / Open Questions

- **`ColumnTypeUnresolved` is a live, catalogued code; its emission is not yet wired.** The code is normative and catalogued in `diagnostics.md`: it fires at a projection whose contributed column type the rules below cannot resolve from a present, well-formed signature (the schema-layer instance of the no-silent-`Unknown` rule). In practice the function-derived inference gaps that previously produced silent unknowns are closed — struct-spread `.*` expansion, `TableExpr` schema propagation through CTEs/subqueries, and nested-call body CTEs all resolve — so a column produced by a *well-formed* function signature does not resolve to `Unknown` today. The one remaining way a function contributes an `Unknown` column is a *malformed* signature (an unrecognized type name nested in the return annotation, e.g. a struct field type); that is a declaration defect reported at its origin via `InvalidFunctionTypeRef` (`functions.md` §Semantics 8), which makes the caller's column `Propagated` and correctly silent. The residual case the code names — a well-formed signature a particular call still cannot type — does not yet arise, so emitting `ColumnTypeUnresolved` for it is a remaining implementation gap to close when such a case appears (Track D-misc), not a reserved/unminted code. Tracked in `docs/plans/20260528-struct-field-type-validation.md` (declaration-side enforcement) and `docs/plans/20260527-function_schema_inference.md` (history).
- **Single-field projection `smelt.functions.<f>(...).field` is not supported.** The parser has no field-postfix syntax on a function call — only `.*` exists via `SMELT_PATH_CALL_STAR`. To access a single field, project the whole struct with `.*` and reference the field downstream. Tracked in `docs/plans/20260527-function_schema_inference.md`.
- **Row-tail (`..r`) struct returns are not expanded by `.*` at the schema layer.** Only closed `Expr<Struct<{f1: T1, …, fN: TN}>>` returns (no tail marker) are expanded. A row-tail return contributes zero columns from the spread at the schema layer; the `.*` passes through to generated SQL verbatim until codegen and schema expansion are unified. Tracked in `docs/plans/20260527-function_schema_inference.md`.

## References

- **Code**:
  - `crates/smelt-db/src/type_inference/function_call.rs` — `infer_smelt_path_call_type` (scalar return resolution), `resolve_struct_return_type` (struct-return scalar type)
  - `crates/smelt-db/src/queries/schema.rs` — `SalsaRefSchemaProvider::resolve_smelt_path_call_schema` / `resolve_table_ref_schema` (`TableExpr` FROM-schema resolution), `typed_model_schema` / `resolved_model_schema` (model output schema, row extensions)
  - `crates/smelt-db/src/function_body_check.rs` — `infer_tableexpr_return_schema`, `register_join_alias_schemas`, `extract_function_body_cte_schemas`
  - `crates/smelt-cli/src/lib.rs` — `init_db` (function discovery into the workspace; parity invariant 3)
  - `crates/smelt-db/src/lib.rs::DiagnosticCode` — diagnostic surface (`ColumnTypeUnresolved` catalogued; emission gap tracked in Known Divergences)
- **Tests**:
  - `crates/smelt-cli/tests/type_command_function_returns.rs` — scalar function-return inference via the `type` discovery flow (parity regression for invariant 3)
  - `crates/smelt-cli/tests/example_diagnostics.rs` — zero-diagnostic gate over example workspaces
  - `crates/smelt-lsp/tests/example_workspaces.rs` — real-Backend zero-diagnostic gate (catches asymmetric-discovery)
- **User docs**:
  - `docs-site/docs/reference/language.md` — `smelt.functions` call surface and `smelt.as_struct`; the `.*` struct-spread schema-projection surface
- **Plans (history)**:
  - `docs/plans/20260422-smelt-functions.md` — function surface, tier model, `TableExpr` return-schema inference
  - `docs/plans/20260519-functions-meta-1-call-expansion.md` — struct-returning and named-arg call expansion (codegen layer)
  - `docs/plans/20260519-functions-meta-2-from-alias.md` — `TableExpr` FROM-position derived-table aliasing
  - `docs/plans/20260519-functions-meta-gaps.md` — tracker for the divergences above
- **Related specs**:
  - `docs/specs/types.md` — `DataType` vocabulary, the `Unknown` reason-discriminant, the strict-by-default / no-silent-`Unknown` doctrine, `TableExpr` row polymorphism
  - `docs/specs/gradual_typing.md` — tier dispatch, call-site return resolution, single-primary-span error contract
  - `docs/specs/functions.md` — declaration grammar, frontmatter, cycle rule
  - `docs/specs/expansion.md` — AST-level call expansion and codegen this spec's schema layer must agree with
  - `docs/specs/architecture.md` — workspace loading parity rule (invariant 3)
  - `docs/specs/meta_language.md` — generator-emitted and reflected schemas, HOF values in data position (out of scope here)
