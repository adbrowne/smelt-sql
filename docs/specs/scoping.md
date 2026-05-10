---
feature: scoping
status: experimental
last_reviewed: 2026-05-05
owners: [andrew]
---

# Scoping

> **Scope.** Normative spec for name resolution inside `smelt.define` bodies: the `Expr<T, ctx>` annotation grammar, parameters-first lookup ordering, the no-overlap rule and its escape hatches, and splice-point context inference. Surrounding specs: `functions.md` (declaration grammar, frontmatter, function-level diagnostics), `types.md` (fragment sorts, type vocabulary, row polymorphism), `gradual_typing.md` (Tier 1/2/3 dispatch).

## Surface

### Context-binding annotation grammar

A fragment-typed parameter that can carry column references may declare a **context** — the name of a sibling `TableExpr` parameter or a CTE defined in the function body whose schema scopes the parameter's columns:

```
Expr<T, ctx>
SelectItems<Kind, ctx>
OrderSpec<ctx>
```

- The `ctx` position appears after the type/kind position. Both grammar shapes are catalogued in `types.md`.
- `ctx` is a bare identifier. It must name **either**:
  1. A sibling parameter of sort `TableExpr` (or `TableExpr<{…}>`) declared on the same `smelt.define`, or
  2. A CTE declared inside the body (a `WITH ctx AS (...)` binding visible at the splice point).
- An identifier in the `ctx` slot is disambiguated from a type name by lookup: if it matches a `TableExpr` parameter or a body CTE, it is a context binding; otherwise it is a type reference and standard type-resolution rules apply.
- Context bindings are **optional**. The compiler infers a parameter's context from its splice point in the body (see Semantics, "Context inference").

### Bare-identifier surface inside a body

Inside a `smelt.define` body, bare identifiers (no qualifier) are resolved by the rules in Semantics, "Resolution order". The user-visible surface is:

- A bare identifier matching a parameter name resolves to the parameter, regardless of whether a column of the same name exists in any FROM-scope table.
- A bare identifier not matching any parameter resolves through standard SQL FROM-scope rules: CTE columns, then `TableExpr` parameter schemas, then the upstream model/source schemas they reach.
- Qualified identifiers (`alias.column`) bypass parameter lookup entirely — parameters are always bare names.
- Bare column references from a `TableExpr` parameter are accepted when **unambiguous**. If only one `TableExpr` in scope exposes a column called `revenue`, `revenue` resolves to that column.

### Diagnostic codes

User-visible codes anchored to scoping. Full descriptions live alongside `DiagnosticCode` in `crates/smelt-db/src/lib.rs`.

| Code | Triggered by |
|---|---|
| `UnknownIdentifier` | A bare identifier in a body resolves to no parameter, no in-scope CTE column, no `TableExpr`-parameter column, and no upstream column. |
| `ParameterShadowsColumn` | An `Expr<T>`-kinded parameter name overlaps a column in a sibling `TableExpr`-parameter's caller schema. Warning severity — body still typechecks, parameter wins, user should use `<table>.<col>` to reach the column. |
| `UnknownContext` | The `ctx` identifier in `Expr<T, ctx>` (or a sibling fragment annotation) does not resolve to any sibling `TableExpr` parameter or body CTE. |
| `ContextMismatch` | An explicit `ctx` annotation disagrees with the context inferred from the parameter's splice point. |
| `AnnotationTooWide` | An explicit `ctx` annotation claims access to columns the inferred splice context does not actually expose. |
| `FragmentColumnMissing` | At a call site, a caller-supplied fragment references a column that is not in the parameter's inferred splice context. |
| `FragmentKindMismatch` | At a call site, a caller-supplied fragment is of a lower expression kind than the parameter requires (e.g. scalar passed where `SelectItems<Agg>` is expected). |
| `CteCycle` | A CTE in a body forms a cyclic reference, directly or transitively. |

## Semantics

These rules are normative.

### Resolution order

Bare-name resolution inside a `smelt.define` body proceeds in this order, and stops at the first match:

1. **Lambda parameters** (Phase B). Inside the body of a `fn x => expr` lambda, the lambda parameter `x` is innermost scope and shadows everything else. Lambda parameters are bound by `TypeContext::add_lambda_param` and looked up before any other scope. A lambda parameter always shadows a same-named function parameter, CTE column, or FROM-scope column. To reach a shadowed outer name inside a lambda body, authors must assign the outer binding to an intermediate before the lambda, or use a qualified reference.
2. **Function parameters** (the function's declared parameter list).
3. **CTE columns** for any CTE in scope at the reference site (a `WITH name AS (...)` binding earlier in the body).
4. **FROM-scope columns** contributed by `TableExpr` parameters in scope. A bare column resolves only when **exactly one** `TableExpr` exposes a column of that name; ties are reported as ambiguity (currently surfaced as `UnknownIdentifier` with a hint until a dedicated `AmbiguousColumn` code lands — see Known Divergences).
5. **Upstream model and source schemas** reachable through `TableExpr`-parameter values (e.g. when a `TableExpr` is bound to a `smelt.<path>` referent, the resolved entity's columns are reachable through SQL FROM resolution against the bound argument; this applies uniformly to models, seeds, and sources).

A qualified reference (`alias.column`) skips steps 1–2 and resolves against the named alias's schema directly. This is the explicit escape hatch when a parameter shadows a desired column.

**Lambda parameter scoping rule.** Lambda parameters are purely lexical. Each `fn x => expr` creates a new scope that lasts exactly for the extent of `expr`. Nested lambdas (currently rejected by `LambdaArityNotSupported` in v1) would each push their own scope. The implementation uses `TypeContext::add_lambda_param` (which mutates a clone of the outer context) and discards the clone after the lambda body walk.

### Parameter shadow warning

If an `Expr<T>`-kinded parameter `p` shares a name with **any** column of any sibling `TableExpr`-parameter's caller schema, the compiler must emit `ParameterShadowsColumn` (Warning) anchored at `p`'s declaration. The body still typechecks — by the parameters-first rule, `p` wins — but the warning alerts the author that bare `p` will not reach the same-named column. The user qualifies (`<table>.p`) to access the column. (`TableExpr`-kinded parameters do not themselves participate in shadow detection: they *become* the FROM scope, they do not shadow it.)

### No-overlap rule

When a function body exposes columns from multiple tables to a caller-provided fragment (typically by joining several `TableExpr` parameters and CTEs), the **column names visible to that fragment must be unique**. There is no union-context type — `Expr<Boolean, source | customers>` is not part of the surface. (Research §16 #4.)

When the same column name is reachable through more than one path at a splice point, the function body is ill-formed for that splice. Authors must use one of three escape hatches:

1. **Explicit CTE rename.** Wrap the joined sources in a body CTE that aliases the colliding columns to unambiguous names, and bind the splice context to that CTE:
   ```sql
   WITH enriched AS (
     SELECT s.*, c.segment AS customer_segment, p.category AS product_category
     FROM source s LEFT JOIN customers c ON … LEFT JOIN products p ON …
   )
   SELECT enriched.*, extra_cols FROM enriched
   ```
   Here `extra_cols: SelectItems<enriched>` sees a flattened, collision-free schema.
2. **Typed `TableExpr` parameter.** Push the disambiguation into the call site by typing the parameter as `TableExpr<{…}>` with the exact required columns:
   ```sql
   smelt.define summarize(source: TableExpr<{region: Text, amount: Numeric, ..}>) -> TableExpr AS (…)
   ```
3. **`smelt.as_struct(alias EXCEPT …)`** for compile-time struct namespacing of multiple joined tables. See `functions.md` for the call surface and `AsStructUnsupportedBackend`. Strategy 3 is recommended only when struct support is universal across the function's declared backends; until `smelt.as_struct` is finalised, Strategies 1 and 2 are the v1 path. See Known Divergences.

### Context inference

The compiler infers a fragment-parameter's **splice context** by tracking where the parameter is used in the body:

- The context is the schema visible at the parameter's splice point in the body (the SELECT list at a `SELECT … metrics …`, the predicate scope at `WHERE filters`, etc.).
- When a parameter is spliced in **more than one** location, its inferred context is the **intersection** of the schemas at each splice point — only columns present in all locations with the same name and compatible type are exposed to the fragment. This is the safe default: a caller-supplied fragment can reference only columns guaranteed to exist wherever it is used.
- An explicit `Expr<T, ctx>` annotation must agree with the inferred context. Annotation serves two roles: documentation in the signature and validation against drift. The annotation is never authoritative on its own — the body is.

### Annotation-versus-inference rules

When both an explicit `ctx` annotation and an inferred splice context exist for the same parameter, the compiler must reconcile them:

- If the named `ctx` resolves to no sibling `TableExpr` parameter and no body CTE, the compiler emits `UnknownContext` at the annotation's `TypeRef` span.
- If the named `ctx` resolves successfully but its schema disagrees with the inferred splice context, the compiler emits `ContextMismatch` at the annotation's `TypeRef` span. Disagreement means the annotated context exposes a different set of columns than the inferred splice point would expose.
- If the annotation claims access to columns that the inferred splice context does not actually expose — i.e. the annotation is **wider** than what the body's splice points can deliver — the compiler emits `AnnotationTooWide` at the call-site argument expression. The annotation cannot promise more than the body can supply.
- If the inferred and annotated contexts agree on column membership, no diagnostic fires; the explicit annotation is preserved as documentation.

### Call-site fragment validation

At a `smelt.<path>(...)` call site (whether arguments are inline or supplied via `PASSING`; see `functions.md`), each caller-provided fragment is validated against the parameter's inferred splice context:

- A column reference inside the fragment that is not present in the splice context emits `FragmentColumnMissing` at the offending column reference.
- A fragment whose synthesised expression kind is **lower** than the parameter's required `Kind` (e.g. a bare scalar passed for `SelectItems<Agg>`) emits `FragmentKindMismatch` at the argument expression. The kind ladder is `Scalar <: Agg <: Window` (see `types.md`).
- Fragment-kind validation is independent of column-context validation; both run.

### CTE rules

CTEs declared inside a `smelt.define` body participate in scoping as follows:

- A CTE name is in scope from the start of its declaration to the end of the body, except inside its own definition (no recursive CTE references in v1).
- CTE schemas are computed using the same schema inference applied to models and function calls; a CTE that selects from a `smelt.<path>(...)` call whose output schema cannot be determined at body-check time is marked **opaque**, and bare-column lookups against it return an `Unknown`-typed result rather than `UnknownIdentifier`.
- A cyclic CTE graph (`A` references `B` references `A`, directly or via `*`-expansion) emits `CteCycle` anchored at every CTE declaration participating in the cycle.
- A CTE name is also a valid `ctx` identifier in fragment annotations — the parameter's columns are scoped to that CTE's output.

### Interactions with adjacent specs

- **Fragment-sort vocabulary, kind ladder, generics, row variables** — see `types.md`. This spec assumes the `Expr<T[, ctx]>` / `SelectItems<Kind[, ctx]>` / `OrderSpec[<ctx>]` grammar; it does not restate the type system.
- **Three-tier checking and how parameters are seeded into the body's `TypeContext`** — see `gradual_typing.md`. The parameters-first rule applies regardless of tier; what differs across tiers is whether parameter types are declared, inferred per call, or required.
- **Function declaration grammar, frontmatter, `PASSING`, function-level diagnostics** — see `functions.md`. The diagnostics catalogued here are the ones whose root cause is name resolution or context binding; declaration-level codes (`DuplicateParameterName`, `UnknownSmeltFn`, etc.) live in `functions.md`.

## Design

This section captures the load-bearing rationale behind the scoping rules above. Where deeper justification exists, it lives in `docs/research/20260413-smelt-functions.md` §6–§7 and §16, and is cross-linked.

**Parameters resolve before SQL FROM scope.** Parameter names are the function's explicit interface — the author wrote them, and the caller binds values to them. A bare identifier matching a parameter therefore always means the parameter, never a column that happens to share the name. The alternative (FROM-scope-first, with parameters consulted only on miss) was rejected because it makes parameter use brittle: an upstream model adding a `user_id` column would silently rewire every body that referenced the `user_id` parameter, with no diagnostic and no caller-visible signal. Parameters-first inverts the failure mode — the body keeps doing what the author intended, and `ParameterShadowsColumn` (Warning) tells the author about the collision so they can rename the parameter or qualify the column with `<table>.<col>` (research §7, §16 #1).

**No-overlap rule plus three escape hatches, not union or qualified-only.** Bare-column resolution must be unambiguous: when two `TableExpr` parameters in scope expose the same name, the body is ill-formed and the author must pick one of three escape hatches — a CTE that aliases the colliders, a typed `TableExpr<{…}>` parameter that pushes disambiguation to the call site, or `smelt.as_struct` for compile-time struct namespacing. Two alternatives were rejected. Union contexts (`Expr<T, a | b>`) reintroduced SQL's join-ambiguity problem with no clean disambiguation rule; the spec would have to define one and the surface would expand to carry it. Qualified-only resolution (always require `alias.col`) preserves the SQL feel poorly — row-polymorphic helpers like `add_margin(source: TableExpr) -> TableExpr AS (SELECT source.*, revenue - cost AS margin FROM source)` rely on bare references, and forcing qualification everywhere would lose that ergonomics. The three hatches preserve flexibility without the spec needing to specify a union or disambiguation algebra (research §6, §16 #4, #7).

**Multi-splice context is intersection, not union.** When a parameter is spliced in more than one place, the columns available to a caller-supplied fragment are the intersection of the schemas at each splice point — only columns guaranteed to exist everywhere the parameter lands. Union was rejected because it lets a body type-check against a column that exists at *some* splice points but not others; the body compiles, then a specific call expansion fails at codegen because the splice produced SQL that referenced a missing column. Intersection moves that failure from build time back to definition time — a column referenced in the body must exist at every splice site, full stop. The trade-off is that some legal SQL (where a column "happens to" exist in all sites by coincidence) is rejected without explicit annotation; the escape hatch is `Expr<T, ctx>` where the author asserts the wider context they want, validated against the body (research §6, §16 #5).

**`Expr<T, ctx>` annotations are documentation, validated against the body — not authoritative.** The body's splice points are the source of truth for what columns a fragment can reference. Annotations exist to make the contract visible in the signature and to fail-fast on drift: editing the body in a way that narrows or shifts a parameter's effective context produces `ContextMismatch` at the annotation. The alternative (annotations override inference) was rejected because it lets users *lie* about body context — a parameter annotated `Expr<T, source>` whose body actually splices it into a CTE-derived scope would compile, and callers would write fragments referencing columns that the splice cannot deliver, with the failure surfacing far from the cause. Inference-as-authority preserves a single source of truth (research §6, §16 #5).

**`AnnotationTooWide` is an Error, not a Warning.** A `ctx` annotation that claims access to columns the inferred splice context does not actually expose is a correctness bug: it advertises a contract the function cannot honour. The fragment passed by a caller may reference one of the over-claimed columns, and the splice point will then emit SQL that references a column not in scope at that point — a codegen-time or runtime failure far from the lying annotation. Surfacing this at definition time with a hard error keeps the diagnostic anchored to the cause, and matches the broader rule that signatures must not promise more than the body delivers.

**CTE alpha-renaming is deferred to v2.** When a body CTE name collides with a CTE introduced by an outer expansion frame, v1 emits a collision diagnostic rather than alpha-renaming. Alpha-rename is the right fix in principle, but it touches the codegen-time expansion machinery (formalised in `expansion.md`) and the planner's provenance contract — both still being stabilised. A collision diagnostic is enough for v1 ergonomically (authors rename one CTE) and avoids shipping a hygiene mechanism that has to break when expansion semantics shift (research §16 #12).

## Constraints & Invariants

1. **Parameters resolve before any SQL scope.** This is unconditional: there is no body context in which a column shadows a same-named parameter. The compiler exposes this through `TypeContext::lookup_identifier` (parameter map first, then column lookup).
2. **The `ctx` annotation is documentation, not authority.** The body's actual splice points determine what columns a fragment can reference. The annotation is checked against the body and never the other way round.
3. **No union contexts.** `Expr<Boolean, source | customers>` and similar union-context syntaxes are not part of the surface and must not be added without a separate spec change. Multiple tables exposed to one fragment go through the three escape hatches.
4. **Multi-splice context is intersection, not union.** When a parameter is used at multiple splice points, the available columns are the intersection of the schemas. No "best-effort" widening.
5. **Bare-column resolution from `TableExpr` parameters requires unambiguity.** When two `TableExpr` schemas in scope expose the same column name, a bare reference is ill-formed regardless of types — qualification is required.
6. **CTE schemas are computed from the body, not from annotations.** Annotating a fragment with a CTE-derived `ctx` couples signature and body intentionally: changing the CTE's SELECT list may change what callers can reference, which is a behaviour change visible in the function's interface.
7. Out of scope for v1 (intent — preserved here so future plans honour it):
   - Recursive CTEs inside `smelt.define` bodies.
   - Union contexts (`Expr<T, a | b>`).
   - Auto-aliasing of colliding `TableExpr` columns (the no-overlap rule is enforced, not silently fixed).
   - Cross-call inference of a parameter's context (each function checks its own body in isolation).
   - A dedicated `AmbiguousColumn` diagnostic — currently funnelled through `UnknownIdentifier` with hint text. See Known Divergences.

## Known Divergences / Open Questions

- **Bare-column resolution from JOIN aliases inside `TableExpr` bodies.** When a `TableExpr` is provided by the caller as a complex expression (e.g. `smelt.a JOIN smelt.b ON …`), the body's bare-column resolution depends on the alias surface that survives the join. Phase 45 of `docs/plans/20260422-smelt-functions.md` covers the remaining work; until it lands, prefer explicit CTE renames inside the body.
- **CTE alpha-renaming is deferred** (research §16 #12). When a body CTE name collides with one introduced by an expansion frame, v1 emits a collision diagnostic rather than alpha-renaming. This is a hygiene gap, not a soundness gap; see `expansion.md` for the planned v2 fix.
- **`smelt.as_struct(...)`** as a no-overlap escape hatch is partially landed: the grammar parses and `AsStructUnsupportedBackend` is wired, but the full semantic finalisation (Step 8 of the smelt-functions plan, alongside struct row polymorphism) is post-v1. Treat Strategy 3 as design-sketch in v1.
- **Ambiguous bare-column references** (a name reachable through two `TableExpr` parameters) currently surface as `UnknownIdentifier` with a hint rather than a dedicated `AmbiguousColumn` code. Whether to mint a distinct code is open.
- **`fragment_param_kinds` seeding.** The body-check pure function seeds a parameter's declared kind into `TypeContext::fragment_param_kinds` so that bare references to a `SelectItems<Kind>`-typed parameter inside `PASSING` bodies inherit the right kind. The seeding contract is wired but lightly tested; corner cases (e.g. a `SelectItems<Agg>` parameter referenced inside a non-aggregate splice point) may surface as Phase 44b/45 follow-ups land.
- **Diagnostic codes pre-`diagnostics.md`.** Codes listed in this spec are owned here until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)

## References

### Code

- `crates/smelt-db/src/type_inference.rs` — `TypeContext` (`function_params`, `tableexpr_param_schemas`, `fragment_param_kinds`, `opaque_ctes`, `cte_columns`), `lookup_identifier`, `columns_for_qualifier`
- `crates/smelt-db/src/function_body_check.rs` — `check_function_body` (parameter binding into `TypeContext`), `compute_shadow_warnings` (`ParameterShadowsColumn`), bare-column resolution from `TableExpr` parameters, fragment validation at call sites
- `crates/smelt-db/src/lib.rs::DiagnosticCode` — every diagnostic code listed in Surface
- `crates/smelt-types/src/signatures.rs` — `ParamSpec` (parameter sort discrimination via `is_tableexpr_param`)

### Tests

- `crates/smelt-db/src/function_body_check.rs::tests` — body-check unit tests covering parameters-first lookup, shadow warnings, bare-column resolution
- `crates/smelt-db/tests/` — workspace-level scoping tests (context inference, multi-splice intersection, fragment-column validation)
- `examples/test_workspace/functions/` — worked examples exercised by the LSP-diagnostics integration test

### User docs

- `docs-site/docs/concepts/functions.md` and adjacent scoping pages — to be reconciled against this spec via `/smelt:validate scoping`

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — primary implementation plan; Phases 5, 15, 19–21, 22, 44b cover the surface in this spec
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/types.md` — type vocabulary, fragment sorts, kind ladder, row polymorphism
- `docs/specs/functions.md` — declaration grammar, frontmatter, `PASSING`, function-level diagnostics
- `docs/specs/gradual_typing.md` — Tier 1/2/3 dispatch and how parameters seed the body's type context
- `docs/specs/architecture.md` — models-as-functions equivalence

### Research

- `docs/research/20260413-smelt-functions.md` — sections 6 (context bindings, no-overlap rule), 7 (parameters-first scoping), 16 decisions 1, 4, 5, 7
