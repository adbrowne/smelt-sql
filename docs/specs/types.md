---
feature: types
status: experimental
last_reviewed: 2026-06-11
owners: [andrew]
---

# Types

> **Scope.** Normative spec for smelt's type system. Covers the concrete `DataType` vocabulary, the strict-by-default doctrine, type constraints, fragment sorts (`Expr<T>`, `TableExpr`, …), promotion rules, and how all of this applies equally to model schemas and `smelt.define` function signatures. Adjacent: `architecture.md`, `incremental_models.md`.

## Surface

### `DataType` vocabulary

The single SQL-type vocabulary. Users write these names in `CAST(... AS T)`, sources YAML, and `smelt.define` annotations. Engine-specific spellings parse into this vocabulary; `to_backend_sql()` is the only path that emits an engine-specific name on output.

| Family    | Types |
|-----------|-------|
| Boolean   | `Boolean` |
| Numeric   | `SmallInt`, `Integer`, `BigInt`, `Float`, `Double`, `Decimal(p, s)` |
| String    | `Varchar(max?)`, `Char(len)`, `Text` |
| Binary    | `Blob` |
| Temporal  | `Date`, `Time`, `Timestamp [WITH TIME ZONE]`, `Interval` |
| Composite | `Array(T)`, `Struct({field: T, …})`, `Map(K, V)` |
| Internal  | `Null`, `Unknown` (compiler-only — surface only via diagnostics; `Unknown` carries a reason — see §"Strict-by-default doctrine") |

Aliases the parser accepts (canonicalised on input): `INT`/`INT4` → `Integer`; `INT2`/`TINYINT` → `SmallInt`; `INT8`/`LONG` → `BigInt`; `INT16`/`HUGEINT` → `BigInt`; `REAL`/`FLOAT4` → `Float`; `STRING` → `Text`; `TIMESTAMPTZ` → `Timestamp WITH TIME ZONE`; `BOOL` → `Boolean`; `BYTEA`/`BINARY` → `Blob`.

### Sources YAML

```yaml
sources:
  <source>:
    tables:
      <table>:
        columns:
          - name: <id>
            type: <DataType-string>      # e.g. INTEGER, DECIMAL(10,2)
            description?: <text>
```

`type` strings use the vocabulary above (or any accepted alias).

### VALUES-derived tables

A `(VALUES (e₁, e₂, …), …) AS alias(c₁, c₂, …)` derived table produces a typed schema using the following rules:

- **Column-wise LUB.** Each column's type is the least upper bound of the corresponding element across all rows, computed by the numeric promotion chain (§"Numeric promotion chain") and the string/temporal family rules (§"String unification", §5).
- **Alias column list.** When `alias(c₁, c₂, …)` provides an explicit column list, the inferred types are bound to those names positionally. When the list is omitted, the columns are named `col1`, `col2`, … (1-indexed). The `AS` keyword is optional: `(VALUES …) t(c₁, …)` binds the column list identically to `(VALUES …) AS t(c₁, …)`.
- **Empty VALUES (zero rows).** A `VALUES` clause with no rows produces no columns. The compiler emits `EmptyValuesClause` (§"Diagnostic codes") anchored at the VALUES clause span.

### `smelt.define` type annotations

Parameter and return positions accept fragment sorts:

| Sort | Form | Example |
|------|------|---------|
| Scalar expression | `Expr<T>` | `Expr<Integer>`, `Expr<Numeric>`, `Expr<Any>` |
| Aggregate | `AggExpr<T>` | `AggExpr<BigInt>` |
| Window-bearing | `WindowExpr<T>` | `WindowExpr<Double>` |
| Table | `TableExpr` or `TableExpr<{col: T, …}>` | `TableExpr<{user_id: Text, ts: Timestamp}>` |
| Select list | `SelectItems<Kind[, ctx]>` | `SelectItems<Agg, base>` |
| Open struct value | `Expr<Struct<{f: T, …}>>` | `Expr<Struct<{ts: Timestamp, ..r}>>` |
| Meta list | `List<T>` | `List<Expr<Numeric>>`, `List<TableExpr>`, `List<List<Text>>` |
| Lambda | `Lambda<T, U>` | `Lambda<Expr<Integer>, Expr<Boolean>>` |
| Model definition | `ModelDef` | (generator-file bodies only — see below) |

`T` is one of: a concrete `DataType`, a `TypeConstraint` (`Numeric`, `Ordered`, `Any`), or — in built-ins / `smelt.extern` only — a generic parameter (`<T: Constraint>`). Row-tail markers on `TableExpr<{…}>` and `Struct<{…}>`: omitted (closed), `..` (anonymous tail accepted), `..r` (named tail bound). Top-level parameter/return types and `TableExpr<{…}>` row columns may carry a trailing `NOT NULL` qualifier (see §11 "Signature nullability"); struct fields and array element types may not.

`List<T>` is a meta-only sort: it never appears as a `DataType` in a runtime column, and `Array<U>` is its Data-World counterpart. The element type `T` may be any other fragment sort (including a nested `List<U>`) or another meta-only type (`ColumnRef`, `ModelRef`, record types). `List<T>` is **covariant** in `T` — if `S <: T` under the fragment-sort subtyping rules below, then `List<S> <: List<T>`. The runtime witness is `SmeltType::List(Box<SmeltType>)` in `crates/smelt-types/src/signatures.rs`. Full surface and semantics for list literals and spread live in `meta_language.md` §"Lists and spread".

`Lambda<T, U>` is a meta-only sort representing a compile-time function value with input sort `T` and output sort `U`. It is **invariant** in both `T` and `U`: `Lambda<S1, T1> <: Lambda<S2, T2>` holds only when `S1 = S2` AND `T1 = T2`. `Lambda<T, U>` values are produced by `fn param => body` syntax and consumed by higher-order functions (`map`, `filter`, `reduce`). The runtime witness is `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` in `crates/smelt-types/src/signatures.rs`. Full surface and semantics for HOFs and lambdas live in `meta_language.md` §"Lambdas and higher-order functions". `Lambda<T, U>` is meta-only — it is not user-writable as a `smelt.define` parameter sort or return type, and is constructed only at HOF positional argument positions; the `LambdaInForbiddenPosition` diagnostic enforces this.

`ModelDef` is a **meta-only closed record type** representing a pending model emission inside a generator file. It is analogous to `ColumnRef`, `ModelRef`, and `SourceRef` — a reflection-witness record — but unlike those three, `ModelDef` is **user-constructible** via a record literal. Its closed field set is fixed as:

| Field | Type | Required | Default |
|---|---|---|---|
| `name` | `Text` | yes | — |
| `body` | `TableExpr` | yes | — |
| `materialization` | `Text` | no | `'view'` |
| `tags` | `List<Text>` | no | `[]` |
| `description` | `Text` | no | `''` |

The runtime witness is `SmeltType::ModelDef`; the closed field set lives alongside `COLUMN_REF_FIELDS` / `MODEL_REF_FIELDS` / `SOURCE_REF_FIELDS` in `crates/smelt-types/src/signatures.rs` as `MODEL_DEF_FIELDS`. `ModelDef` is intentionally distinguishable from a structurally-equal `Struct<{…}>` — the type identity carries the restriction that construction is only valid inside a generator-file body (a file with `generates: models` frontmatter); a `ModelDef` literal anywhere else emits `ModelDefOutsideGeneratorFile`. `List<ModelDef>` is the required return type of a generator-file body. `ModelDef` values never reach the database engine; they are consumed entirely at meta-evaluation time. Full semantics live in `meta_language.md` §"Multi-model production".

`Kind` ∈ `{Scalar, Agg, Window}`. `ctx` is the name of a sibling parameter whose schema scopes the items.

Trailing variadic `...` on the final argument is allowed in built-ins / `smelt.extern` only.

### Diagnostic codes (user-visible)

Type-related codes from `crates/smelt-db/src/lib.rs::DiagnosticCode`. These are the spec's checkable anchor points:

`TypeMismatch`, `CannotInferType`, `UnknownCastType`, `UnrecognizedFunction`, `SourceTypeError`, `WindowInScalarContext`, `AmbiguousColumn`, `UndeclaredColumn`, `ColumnTypeUnresolved`, `ArgTypeMismatch`, `FunctionBodyTypeMismatch`, `ReturnTypeMismatch`, `InvalidFunctionTypeRef`, `MissingArgument`, `RowRequirementUnsatisfied`, `FragmentColumnMissing`, `AnnotationTooWide`, `FragmentKindMismatch`, `ParameterShadowsColumn` (warning), `AliasColumnArityMismatch`, `EmptyValuesClause`, `DecimalPrecisionOverflow`.

`DecimalPrecisionOverflow` — emitted when a decimal arithmetic expression or UNION coercion computes a result precision `p' > 38`. Anchored at the operator token (arithmetic) or UNION keyword span (UNION coercion). Recovery: the result degrades to `Unknown` (reason `Unresolved`). The user must cast one or both inputs to a narrower `Decimal(p, s)` before the operation.

`AliasColumnArityMismatch` — emitted when the alias column list in `(VALUES …) AS t(c₁, c₂, …)` or `WITH cte(c₁, c₂, …) AS (SELECT …)` has a different length from the underlying relation's actual column count. Anchored at the `ALIAS_COLUMN_LIST` span (the parenthesised name list). Recovery: applies alias names positionally up to whichever list is shorter; any remaining columns retain their inferred names (CTEs over a SELECT body) or fallback `colN` names (VALUES derived tables).

`EmptyValuesClause` — emitted when a `(VALUES …)` derived table contains no rows and therefore cannot produce a typed schema. Anchored at the VALUES clause span. The empty-VALUES form is not yet reachable through hand-authored SQL syntax; it is reserved for future meta-language surfaces (e.g. a list splice that yields zero rows at compile time).

`ColumnTypeUnresolved` is the schema-layer instance of the no-silent-`Unknown` rule (§"Strict-by-default doctrine"): it fires when a resolved column's type degrades to `Unknown` for a compiler-resolvable reason rather than a genuinely dynamic one. The schema-propagation rules that produce it for `smelt.functions.*`-derived columns live in `function_schema_inference.md`.

### Hover

The LSP renders types using this vocabulary. `Text` displays as `Text`, not `Varchar`. Constraints display by name (`Numeric`, `Ordered`). Unknown types display as `Unknown`. Non-nullable columns display with a `NOT NULL` suffix (`Integer NOT NULL`); nullable columns display the bare type — display notation matches the writable annotation syntax (§11 "Signature nullability"). Hover and diagnostics share one canonical type renderer so tracked axes are never silently dropped from user-facing output.

## Semantics

### 1. Strict-by-default doctrine

Cross-family operations must produce `Unknown` and emit a diagnostic. The compiler must not synthesise an implicit cast across families. This is settled doctrine, not a configurable mode.

- `Integer + Varchar`, `Boolean + Integer`, `42 + '3'` → `Unknown` + `TypeMismatch`.
- Mixed-type array literals (e.g. `[1, 'hello']`) → error.
- UNION / INTERSECT / EXCEPT branches with incompatible families → `Unknown` + diagnostic.

The user must write an explicit `CAST` to bridge families. The LSP provides quick-fixes; committed code is strict.

**No silent `Unknown`.** The cross-family rule above is one instance of a general invariant: every `Unknown` that surfaces in a resolved column schema or expression type **must** be accompanied by a diagnostic. `Unknown` is the compiler's "we already told you about this" type, never a quiet fallback. To make this enforceable without a cascade of duplicate errors, `Unknown` carries a **reason**:

| Reason | Meaning | Diagnostic |
|---|---|---|
| `Unresolved` | A compiler-resolvable gap or unbound type the inference failed to determine (cross-family op; an `smelt.functions.*`-derived column the schema rules cannot type). | **Yes**, at the origin (`TypeMismatch`, `ColumnTypeUnresolved`, …). |
| `Dynamic` | A legitimately unknowable type (`Expr<Any>` return, dynamic SQL). Not a defect. | No. |
| `Propagated` | `Unknown` only because an upstream value was already `Unknown` and reported. | No — reporting is origin-only. |

Reporting is **origin-only**: a `Propagated` `Unknown` re-emits nothing, preserving the single-primary-span contract (`gradual_typing.md`). Only `Unresolved` at its origin produces a diagnostic; `Dynamic` never does. There is no opt-out: as with cross-family strictness, the diagnostics fire by default and are not configurable.

### 2. Numeric promotion chain

The least-upper-bound (LUB) for promotion (in expressions, UNION, generic argument inference) follows a single linear chain:

```
SmallInt < Integer < BigInt < Decimal < Double
```

`Float` collapses into `Double` for promotion purposes. When the LUB involves `Decimal`, the specific `(p, s)` is governed by §15 "Decimal arithmetic" — integer-family members are lifted to their natural decimal equivalents there, and the growth formula or UNION coercion rule determines the result `(p, s)`.

### 3. Integer division is truncating

`Integer / Integer → Integer`. The integer family is preserved through `/` (no widening to Double). Backend SQL generation may insert casts to match an engine's native semantics, but the smelt-internal type is the truncating result.

### 4. String unification

`Text`, `Varchar(_)`, `Char(_)` are interchangeable for type-equality (`normalize()` collapses `Text ↔ Varchar(None)`). String operations discard length annotations. String functions (`UPPER`, `SUBSTRING`, `||`, …) return `Text`. Backend output emits `VARCHAR` for engines without a `TEXT` type via `to_backend_sql()`.

### 5. Canonical built-in returns

Built-in SQL function return types are taken from the canonical registry in `crates/smelt-types/src/signatures.rs::BuiltinRegistry` and must be enforced via `CAST` in generated SQL. Examples:

- `SUM(Integer | BigInt | SmallInt) → BigInt`
- `SUM(Double | Float) → Double`
- `SUM(Decimal(p, s)) → Decimal(38, s)` — scale preserved; precision widened to the maximum within the 38-digit limit. ByDesign divergence from Spark (see §15).
- `AVG(any numeric) → Double`
- `MIN(T) → T`, `MAX(T) → T` for any `T: Ordered` (input type preserved; nullability §11 applies — empty group returns `NULL`).
- `COUNT(*) → BigInt` (non-nullable — guaranteed by SQL semantics).
- `COUNT(expr) → BigInt` (non-nullable).
- `CEIL(Double) → Double`, `CEIL(Decimal(p,_)) → Decimal(p, 0)`
- `SIGN(any numeric) → SmallInt`
- `ABS(Decimal(p, s)) → Decimal(p, s)` — preserves precision and scale. `ABS(other numeric T) → T`.

Engine-native precision is opt-in via the backend namespace (`postgres.sum(...)`); using it marks the model as non-portable. Timezone-sensitive function returns (`NOW`, `CURRENT_TIMESTAMP`, `MAKE_TIMESTAMPTZ`, `DATE_TRUNC`) are governed by §16.

**Aggregate non-nullability via `COALESCE`.** A `COALESCE(<agg>, <literal>)` expression where `<literal>` is a non-null literal whose type matches `<agg>`'s return type is non-nullable. This is the canonical idiom for "empty-group becomes 0" (e.g. `COALESCE(SUM(amount), 0)`); the column-level `nullable: bool` flag flips to `false` after the COALESCE wrap. Type matching follows §11 nullability rules — this section only specifies the non-nullability outcome, not the equality predicate.

### 6. Type constraints

- `Numeric` ≡ `{SmallInt, Integer, BigInt, Float, Double, Decimal(*)}`. `Boolean` excluded.
- `Ordered` ≡ `Numeric ∪ {Text, Varchar(*), Char(*)} ∪ {Date, Time, Timestamp(*), Interval} ∪ {Boolean, Blob}`. Composite types (`Array`, `Struct`, `Map`), `Null`, and `Unknown` are non-members.
- `Any` accepts every concrete `DataType`.
- `Numeric ⊂ Ordered` is structural; callers do not restate the bound.

A constraint is satisfied iff the actual type is a member. Constraints are **not** coercion: passing `Integer` to a parameter declared `Expr<Double>` is an error (no implicit promotion across concrete types).

### 7. Fragment sort subtyping

The subtype relations include two linear chains for expression and select-item sorts:

```
Expr<T>             <:  AggExpr<T>             <:  WindowExpr<T>
SelectItems<Scalar> <:  SelectItems<Agg>       <:  SelectItems<Window>
```

And two one-way closed-record rules for wide-reflection meta types:

```
ModelRef   <:  TableExpr
SourceRef  <:  TableExpr
```

**`ModelRef <: TableExpr`.** A `ModelRef` value lifts to a `TableExpr` wherever a `TableExpr` is required — reducer-`union_all` arguments, `smelt.columns_of` arguments, and FROM-clause splice positions. The lifted `TableExpr` is the same table representation that `smelt.<model-path>` resolves to for that model. The rule is **one-way**: the reverse direction (`TableExpr → ModelRef`) does not exist; only values originating from `smelt.models.*` accessors are `ModelRef`-typed.

**`SourceRef <: TableExpr`.** The same lifting rule applies to `SourceRef` values produced by `smelt.sources.*` accessors. The lifted `TableExpr` is the same table representation that the source's `smelt.<source-path>` resolves to.

**List covariance applies.** Because `List<T>` is covariant in `T` (§"smelt.define type annotations"), `List<ModelRef> <: List<TableExpr>` and `List<SourceRef> <: List<TableExpr>` follow automatically. This means `reduce(smelt.models.with_tag('cohort'), union_all)` requires no explicit projection — the list element type lifts to `TableExpr` through the single rule above.

`TableExpr`, `ModelRef`, `SourceRef`, and `OrderSpec` are otherwise unrelated to the expression chain. Splice points enforce a kind ceiling:

| Position | Accepted kinds |
|----------|----------------|
| `WHERE`, `GROUP BY`, `ON` | `Scalar` only |
| `HAVING` | `Scalar`, `Agg` |
| `SELECT` (no GROUP BY) | `Scalar`, `Agg`, `Window` |
| `SELECT` (with GROUP BY) | `Scalar`, `Agg` |
| `QUALIFY` | `Scalar`, `Agg`, `Window` |

### 8. Generics inference (built-ins / `smelt.extern` only)

For each type parameter `T` in a signature, the checker collects every position where `T` appears: argument positions, plus the expected return type when the call is in checking mode (`TypeContext.expected_return`). Then:

- If `T`'s declared constraint has a promotion chain (only `Numeric` in v1), bind `T` to the LUB of the collected positions under that chain.
- Otherwise, every position must unify to the same concrete type (after `Text ↔ Varchar` normalization). A mismatch is a `TypeMismatch` / `ArgTypeMismatch` at the first conflicting position.

After binding, the declared constraint is discharged (e.g. `T: Ordered` rejects `T = Map<…>`).

### 9. Variadics

Trailing `...` marks the final argument position as variadic. Variadics expand to N positions for inference, all sharing the same `T` if one is declared. Variadics are positional only — `name => value` syntax does not apply. At most one variadic per signature; user-defined `smelt.define` functions are not variadic in v1.

### 10. Bidirectional checking

- **Call sites**: declared parameter types are pushed into arguments (checking mode).
- **Function bodies**: parameter types seed the type context; the body synthesises bottom-up. If a return type is declared (Tier 3), it provides a checking target — a body that synthesises a different type emits `ReturnTypeMismatch`.
- **Row variables** on `TableExpr<{…, ..r}>` and `Expr<Struct<{…, ..r}>>` unify locally per call site against the concrete caller schema.
- Errors are local; row variables never appear in user-facing diagnostics — they bind first, then any error reports the concrete fields.
- Tier 1 (unannotated) functions are checked by binding parameter names to the caller's argument types in the type context and re-checking the body. Tier 2/3 functions are checked in isolation under their declared signatures.

### 11. Nullability

Columns carry `nullable: bool` in `TypedColumn`.

**Sound-upper-bound contract.** `nullable: false` is a guarantee: the column cannot contain `NULL` in any row, for any input data satisfying the declared source schemas. `nullable: true` means only "may contain NULL". When inference cannot establish the guarantee, it must answer `nullable: true`. Claiming `nullable: false` for a column that can hold NULL is a soundness defect; claiming `nullable: true` for a column that provably cannot is acceptable imprecision. Because the contract is one-sided, the rules below enumerate the **only** ways an inferred column or expression may be non-nullable; anything not covered defaults to nullable.

- **Non-nullable origins:** non-NULL literals; source/seed columns declared `nullable: false`; `COUNT(*)` and `COUNT(expr)`; `EXISTS`; struct/array literals (the container itself); `COALESCE(…)` with at least one non-nullable argument; `CASE … ELSE …` when all result branches are non-nullable; `CAST` preserves the input's nullability. Scalar operators and functions that are NULL-propagating may claim non-nullable only when every operand is non-nullable.
- **Always nullable (overrides non-nullable inputs):** `SUM`, `AVG`, `MIN`, `MAX` (empty groups → NULL); scalar subqueries; `IN (subquery)`; array subscript (out-of-bounds → NULL); struct field access (conservative); `TRY_CAST`; `NULLIF`; `CASE` without `ELSE`; `LAG`/`LEAD` without an explicit default.
- **Outer joins.** Columns sourced from the null-supplying side(s) of an outer join are nullable in the join's output scope, regardless of declared or upstream-inferred nullability: `LEFT JOIN` — all right-side columns; `RIGHT JOIN` — all left-side columns; `FULL JOIN` — both sides. `INNER` and `CROSS` joins preserve input nullability.
- **Set operations.** A `UNION` / `INTERSECT` / `EXCEPT` output column is non-nullable only if the corresponding column is non-nullable in **every** branch.

**Verification gate.** The contract is verified against engines by a value-based property test: generated queries over generated data (every nullable input column actually populated with NULLs) execute on DuckDB, and no output column smelt infers as `nullable: false` may contain a NULL in any result row. The check must be value-based, not schema-based — DuckDB/Arrow result schemas mark columns nullable indiscriminately, so schema comparison carries no information. Conservative answers need no divergence registry: over-claiming nullable is free under the contract; only a non-nullable claim contradicted by data is a defect.

**Signature nullability.** Type positions that correspond to the column channel accept a trailing `NOT NULL` qualifier: top-level parameter and return types (`Expr<Integer NOT NULL>`, `AggExpr<Numeric NOT NULL>`) and `TableExpr` row columns (`TableExpr<{id: Integer NOT NULL, …}>`). A bare type is nullable — non-null is opt-in, so unqualified signatures keep their meaning. The qualifier is orthogonal to constraints (`Expr<Numeric NOT NULL>` is legal). It is **not** accepted on struct fields or array element types — nested composite nullability is untracked (see Design). Checking rules:

- Subtyping is one-way: non-nullable `T` <: nullable `T`. A non-nullable argument satisfies a nullable parameter; the reverse is an error.
- A `NOT NULL` parameter requires a non-nullable argument at the call site; violations emit `ArgTypeMismatch`. Inside the body, the parameter binds non-nullable in the type context.
- A `NOT NULL` return type requires the body to synthesise a non-nullable result; violations emit `ReturnTypeMismatch`.
- A `NOT NULL` row column in `TableExpr<{…}>` requires the caller's column to be non-nullable; violations emit `RowRequirementUnsatisfied`.

No new diagnostic codes: nullability mismatches reuse the codes above with nullability-aware messages.

### 12. Models are functions

A model `m` (a bare `SELECT` in some `.sql` file) is equivalent to a `smelt.define m(...)` whose `TableExpr` parameters default to `smelt.<path>` references resolved against the workspace (the universal addressing scheme — see `architecture.md` §"Resolution"). The materialization decision (`table` / `view` / `ephemeral` / `materialized_view`) is orthogonal to the type system. In particular:

- A model's `smelt.<path>` references — whatever they resolve to (upstream model, source, seed) — contribute `TypedColumn` entries to the body's `TypeContext`, exactly as `TableExpr` parameters do for functions.
- The output of a model is a `ModelSchema` (`Vec<Column>` + row extensions + input constraints), which is the same data a function returning `TableExpr` would produce.
- `TableExpr<{…}>` row-polymorphism applies identically to function parameters and to model input constraints.

### 13. `TableExpr` row polymorphism

- Bare `TableExpr` accepts any caller schema unchanged.
- `TableExpr<{col: Type, …}>` requires every declared column to be present in the caller's schema with a satisfying type (concrete equality after `Text ↔ Varchar` normalization, or constraint satisfaction).
- Closed shape (no tail) tolerates extra caller columns by default in v1 (open-record); use `..` to make this explicit, or `..r` to bind extras to a row variable for use in the body and return type.
- Failures emit `RowRequirementUnsatisfied` at the argument expression; the body check is short-circuited so no cascading diagnostics surface from inside the callee.

### 14. Strict family-rejection examples

| Expression | Result | Diagnostic |
|------------|--------|------------|
| `42 + '3'` | `Unknown` | `TypeMismatch` |
| `TRUE + 1` | `Unknown` | `TypeMismatch` |
| `[1, 'hello']` | `Unknown` (array) | `TypeMismatch` |
| `SELECT 1 UNION SELECT 'a'` | `Unknown` per column | `TypeMismatch` |
| Window function in `WHERE` | flagged | `WindowInScalarContext` |

### 15. Decimal arithmetic

**Portable surface.** `+`, `-`, `*`, and `%` on `Decimal` operands are in the portable surface. Division (`/`) is not — it emits `TypeMismatch` at the operator span (see "Division rejection" below). Unary minus preserves the operand's `(p, s)`.

**Integer lifting.** When an integer-family operand appears in a decimal arithmetic expression, it is lifted to its natural decimal equivalent before the growth formula applies:

| Integer type | Lifted as |
|---|---|
| `SmallInt` | `Decimal(5, 0)` |
| `Integer` | `Decimal(10, 0)` |
| `BigInt` | `Decimal(19, 0)` |

**Arithmetic growth formulas.** Applied after integer lifting; both operands are decimal-family at this point.

| Operator | Result precision `p'` | Result scale `s'` |
|---|---|---|
| `a + b`, `a - b`, `a % b` | `max(p₁ − s₁, p₂ − s₂) + max(s₁, s₂) + 1` | `max(s₁, s₂)` |
| `a * b` | `p₁ + p₂ + 1` | `s₁ + s₂` |

**Decimal LUB (UNION and generic inference).** When two `Decimal(p₁, s₁)` and `Decimal(p₂, s₂)` values are combined via UNION branch coercion or generic-parameter inference (not binary arithmetic), the result is `Decimal(max(p₁-s₁, p₂-s₂) + max(s₁, s₂), max(s₁, s₂))` — the narrowest type that can represent values from either operand without overflow. There is no `+1` carry term; coercion introduces no new digits.

**Compile-time overflow check.** If `p'` computed by either formula (arithmetic or coercion) exceeds 38, the operation emits `DecimalPrecisionOverflow` anchored at the operator token (or UNION keyword) span; the result type degrades to `Unknown` (reason `Unresolved`). The user must cast inputs to narrower decimals before the operation.

**Division rejection.** `Decimal / T` for any numeric `T` is not in the portable surface. The expression emits `TypeMismatch` anchored at the `/` operator. The diagnostic message directs the user to:

1. Cast inputs to `Double` or `Float` for floating-point division (always portable): `CAST(a AS DOUBLE) / CAST(b AS DOUBLE)`.
2. Engine-bound decimal division: not yet available; see Known Divergences.

**Aggregate and unary decimal returns.** These entries own the §5 canonical returns for decimal-typed inputs:

- `SUM(Decimal(p, s))` → `Decimal(38, s)` — summation preserves scale (no new decimal digits), but precision grows; `38` is the maximum within the engine intersection. ByDesign divergence from Spark (`DECIMAL(min(p+10, 38), s)`).
- `AVG(Decimal)` → `Double` — implicit division-by-count can produce fractional digits that cannot be expressed in the input's `(p, s)` portably. ByDesign divergence from Spark (stays in `Decimal`); DuckDB agrees on `Double`.
- `ABS(Decimal(p, s))` → `Decimal(p, s)` — absolute value preserves precision and scale.

### 16. Timezone

**Two-variant type.** `Timestamp` (naive — no timezone offset, no absolute UTC referent) and `Timestamp WITH TIME ZONE` (tz-aware — UTC-normalised internally) are distinct types. The `with_timezone` flag is a value-domain axis: it lives in `DataType`, not in `TypedColumn`, and participates in type equality, promotion, and `CAST` — consistent with the axis-placement rule in §Design. The parser alias `TIMESTAMPTZ` normalises to `Timestamp WITH TIME ZONE` on input.

**Strict mixing rule.** Using a naive `Timestamp` and a `Timestamp WITH TIME ZONE` as branches of the same UNION / EXCEPT / INTERSECT, or as CASE alternatives, is a `TypeMismatch` at the set-operator or CASE keyword span. The user must `CAST` to align variants before the set operation.

**Timezone-sensitive function returns.** These entries override §5 for the listed functions:

| Function | Return type | Notes |
|---|---|---|
| `NOW()` | `Timestamp WITH TIME ZONE` (non-nullable) | DuckDB and Spark both return tz-aware. |
| `CURRENT_TIMESTAMP` | `Timestamp WITH TIME ZONE` (non-nullable) | Same semantics as `NOW()`. |
| `MAKE_TIMESTAMPTZ(y, mo, d, h, mi, s [, tz])` | `Timestamp WITH TIME ZONE` (nullable) | The `_TZ` suffix signals tz-aware output. `MAKE_TIMESTAMP` (without TZ) returns naive `Timestamp`. |
| `DATE_TRUNC(part, Timestamp)` | `Timestamp` (nullable) | Tz-axis preserved from input. |
| `DATE_TRUNC(part, Timestamp WITH TIME ZONE)` | `Timestamp WITH TIME ZONE` (nullable) | Tz-axis preserved from input. DuckDB and Spark agree. |

**Arithmetic is tz-transparent.** The timezone axis passes through unchanged:

- `Timestamp +/- Interval → Timestamp`
- `Timestamp WITH TIME ZONE +/- Interval → Timestamp WITH TIME ZONE`

Mixing tz variants in arithmetic (e.g. `Timestamp WITH TIME ZONE - Timestamp`) is a `TypeMismatch` by the same rule as mixing in set operations.

**`AT TIME ZONE` — not in portable surface.** The SQL expression `expr AT TIME ZONE zone_str` — which converts `Timestamp → Timestamp WITH TIME ZONE` and `Timestamp WITH TIME ZONE → Timestamp` — is not yet in the portable surface. See Known Divergences.

**Soundness oracle.** `cargo test -p smelt-db --test type_property_tests` must cover `TimestampTz`-typed input columns; new divergences are either fixed or registered `ByDesign` in `divergences.rs`.

## Design

This section captures the load-bearing rationale behind the type system's shape and the alternatives that were considered and rejected.

**Strict-by-default, no implicit cross-family coercion.** The Semantics §"Strict-by-default doctrine" rule — `42 + '3'` is a `TypeMismatch`, not a silent string-to-int coercion — is the single most user-visible decision in the type system. It is doctrine because every alternative we considered traded near-term ergonomics for far-more-expensive long-term debugging. *Implicit coercion across families* (the SQL-92 / MySQL shape) was rejected because it lets schema drift hide for months: a column that quietly turns from `INTEGER` to `VARCHAR` upstream still type-checks downstream and silently corrupts joins. *Configurable strictness* (a `--strict` flag, or per-project lenient mode) was rejected because once strict diagnostics are optional, large projects negotiate them away and the framework loses its strongest correctness lever. The strict rule is paid back in two ways: the LSP offers one-click `CAST` quick-fixes, and committed code has a single, documented, mechanical answer to "why did this expression infer `Unknown`?".

**The `Unknown` reason-discriminant exists to make "no silent `Unknown`" enforceable without cascades.** The doctrine wants every `Unknown` to carry a diagnostic, but a naive rule ("error on any `Unknown` column") fails two ways: it would flag legitimately-dynamic values (`Expr<Any>`), and it would report one root gap once per downstream consumer (an unresolved upstream column lights up every model that reads it). The three-way reason (`Unresolved` / `Dynamic` / `Propagated`) resolves both: only `Unresolved` at its origin reports, `Dynamic` is silent by design, and `Propagated` suppresses re-emission so the error appears once at its source. *Banning the `Unknown` value outright* was rejected because `Unknown` is load-bearing as the inference lattice's bottom — the signal worth surfacing is "an `Unknown` we should have resolved", which is exactly what the discriminant encodes. *An opt-in `strict_types` flag with a warning→error ramp* was rejected for the same reason configurable strictness was rejected above: an optional correctness diagnostic gets negotiated away. The migration cost (existing silent `Unknown`s, e.g. unexpanded struct-spread schemas) is paid by **sequencing** — close the inference gaps so the legitimate cases resolve, then the residual `Unresolved` columns surface by default — not by a permanent opt-out.

**Single `DataType` vocabulary across all backends.** `DataType` is one enum in one crate (`smelt-types`); backend-specific names (`HUGEINT`, `STRING`, `TIMESTAMPTZ`) parse into the enum on input, and `to_backend_sql()` is the only path that emits an engine-specific name. *Per-backend type vocabularies* (one `DataType` for DuckDB, another for Spark, another for Postgres) was rejected because cross-backend reasoning — incremental models that read DuckDB and write Spark, function signatures portable across engines, type-aware diagnostics in the LSP — would require a translation layer at every boundary, and translation layers are where correctness goes to die. The trade-off is that adding an engine-native type that no other backend supports requires either a new `DataType` variant (semi-permanent) or a `<backend>.<type>(...)` opt-in (the `postgres.sum(...)` shape from §"Canonical built-in returns"). That cost is paid by the few projects that need it, not by the ecosystem as a whole.

**Axis placement: value-domain axes live in `DataType`; column-population axes live in `TypedColumn`.** Axes that change what values can exist — decimal precision/scale, timezone-awareness, varchar length — are part of the type proper and participate in promotion, unification, and `CAST`. Axes that describe properties of a column's data over an unchanged value domain — nullability today, collation prospectively — live beside the type in `TypedColumn`. This matches SQL's own model (`NOT NULL` is a column constraint, not a type) and keeps nullability out of every type-equality and promotion rule. *`Nullable<T>` in the `DataType` enum* was rejected: it forces every `match` on `DataType` and every unification/promotion rule to answer "what about the wrapper?", an invasive change buying little since `TypedColumn` already flows everywhere inference goes. The accepted cost is that **composite types erase the column channel**: `Struct` fields and `Array` elements carry only a `DataType`, so nested nullability is untracked and nested access is conservatively nullable (§11). This is also the cross-engine intersection — DuckDB and Postgres provide no syntax to declare or enforce `NOT NULL` on struct fields or array elements, so nested positions are always-nullable there; only Spark tracks the nested axis (`StructField.nullable`, `ArrayType.containsNull`, `MapType.valueContainsNull`). If Spark-grade nested precision is ever wanted, the extension point is the composite `DataType` variants themselves (per-field/element nullability flags, Spark's shape), not a relocation of the column-level flag. Collation's placement is tentative until its own design cycle; its SQL coercibility rules suggest the column channel, like nullability.

**Decimal arithmetic: growth formula choice, division exclusion, and AVG→Double.** `+`, `-`, `*`, `%` use Spark's Hive-derived growth formulas — the portable intersection between Spark (exact, 38-capped) and DuckDB (same formulas for non-division ops). Postgres's unbounded exact arithmetic is not reachable within the 38-digit limit; the compile-time overflow check (`p' > 38`) makes precision exhaustion a compile error rather than a runtime surprise. *Division is excluded* because the three engines disagree on the result type family: DuckDB promotes `Decimal / Decimal` to `Double` to avoid infinite-precision growth; Spark stays in `Decimal` with a Hive formula; Postgres returns unbounded `NUMERIC`. No cast sequence produces bit-identical results across all three, so the portable surface refuses the operation. The two current remedies are cast-to-Double (always portable) and engine-bound models (future). A `smelt.divide(a, b, scale => N)`-style stdlib polyfill — the long-term ergonomic answer — requires engine-dispatched function bodies, which are not yet designed; the research doc (`docs/research/20260516-decimal-type-system.md` §7) sketches the shape. *AVG returning `Double`* follows from division: count-based averaging involves implicit division by the row count, which can produce fractional digits that cannot be expressed in the input's `(p, s)` portably across engines; `Double` is the universally-safe answer (DuckDB agrees; Spark diverges). *SUM returning `Decimal(38, s)`* retains scale because summation never introduces new decimal digits — the scale of a sum equals the addends' scale — while the integer part can grow unboundedly; `38` is the maximum within the engine intersection, and DuckDB agrees.

**Timezone mixing is an error, not silent widening.** When a naive `Timestamp` and a `Timestamp WITH TIME ZONE` appear as UNION branches or CASE alternatives, smelt emits `TypeMismatch` rather than silently widening to `Timestamp WITH TIME ZONE`. The alternative — widening — was rejected because naive and tz-aware timestamps carry different semantics: a naive timestamp has no absolute UTC referent, so widening it silently asserts that it does. Real bugs (a `created_at TIMESTAMP` column from one source mixed with a `ts TIMESTAMPTZ` from another) would silently produce a tz-aware result whose UTC interpretation is engine-defined and often wrong. The strict rule follows the same logic as cross-family arithmetic in §1: correctness hazards are errors, not silent promotions. The user resolves this with an explicit `CAST` — `CAST(naive_ts AS TIMESTAMP WITH TIME ZONE)` — which makes the intent auditable. `NOW()` and `CURRENT_TIMESTAMP` returning `Timestamp WITH TIME ZONE` (fixing the prior `with_timezone: false` default) is the motivating correction; the strict mixing rule is what makes the fix load-bearing rather than cosmetic.

**Engine-alias normalisation is a parser concern, not an inference concern.** The aliases `INT`, `INT4`, `STRING`, `BOOL`, `BYTEA`, `TIMESTAMPTZ` are normalised on input by `crates/smelt-types/src/parse.rs`; type inference operates only on canonical `DataType` values. *Carrying alias spellings through inference* was rejected because it doubles the surface every inference rule has to handle ("does `Integer + Int` unify? does `Text + String` round-trip?") with no semantic value — every such pair is the same type. Normalising at the boundary keeps the inference rules clean and means the test surface for type inference doesn't have to enumerate alias permutations.

**Local, bidirectional checking; no global constraint solver.** Each function call's row-variable unification, generic binding, and constraint discharge happens at the call site (§"Bidirectional checking", §"Generics inference"). *Global constraint propagation* (a Hindley-Milner-style solver across the whole workspace) was rejected because workspaces grow to thousands of models and millions of inferred columns, and global solvers do not survive that scale gracefully — incremental recompilation in particular suffers, since one edit can invalidate constraints far away. Local checking means the LSP's "what's the type of this expression?" question always has a fast answer, and parameterised functions feel like local reasoning ("what does `T` bind to here?") not deep magic. The trade-off is that callers must annotate `smelt.define` signatures whose generics cannot be inferred locally — a price we accept (the function form is `smelt.extern` / built-ins; user-defined `smelt.define` is monomorphic in v1).

## Constraints & Invariants

- The `DataType` enum in `crates/smelt-types/src/lib.rs` is the single SQL-type vocabulary; backend-specific names (HUGEINT, STRING, TIMESTAMPTZ) parse into it on input, and `to_backend_sql()` is the only emission path that produces an engine-specific name.
- `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` contain pure functions (no Salsa imports). Salsa queries are thin wrappers — see CLAUDE.md "Pure Function Rule".
- Function call inference is **local**: row-variable unification, generic binding, and constraint discharge all happen at the call site without cross-module constraint solving.
- **No silent `Unknown`.** Every `Unknown` with reason `Unresolved` surfacing in a resolved schema or expression type is accompanied by an origin diagnostic. `Dynamic` and `Propagated` unknowns are diagnostic-free by construction. The set of `Unknown` reasons (`Unresolved`, `Dynamic`, `Propagated`) is closed; adding a reason requires a spec edit.
- **Nullability is a sound upper bound.** No inference rule may produce `nullable: false` outside the enumerated non-nullable origins in §11; the default for any uncovered construct is `nullable: true`. Standing gate: the value-based nullability soundness property tests in `crates/smelt-db/tests/nullability_property_tests.rs`.
- `Numeric ⊂ Ordered` is structural — callers do not need to restate constraints.
- Adding a type to `Ordered` is non-breaking; removing one is breaking.
- Fragment sort subtyping for expression-family sorts (`Expr<T>`, `AggExpr<T>`, `WindowExpr<T>`, `SelectItems<K>`) is linear-only. The two closed-record lifting rules (`ModelRef <: TableExpr`, `SourceRef <: TableExpr`) are the complete set of non-expression-chain subtyping rules; no further branching is permitted without a spec edit. `ModelDef` participates in no subtyping rule — it is neither a subtype nor a supertype of any other sort.
- One canonical built-in registry (per `signatures.rs::BuiltinRegistry`); per-dialect registries are out of scope. Backend availability is a per-function `backends:` property, not a registry split.
- **Out of scope for v1**: nested composite nullability (struct fields, array elements — conservatively nullable); multiple row variables per function; user-defined polymorphism in `smelt.define`; collation tracking on `Text`; engine-bound decimal division (the `/` operator on `Decimal` operands is rejected in portable code — see §15).
- **Standing timezone gate.** `cargo test -p smelt-db --test type_property_tests` must stay green with `TimestampTz` inputs exercised. `NOW()` and `CURRENT_TIMESTAMP` must return `Timestamp WITH TIME ZONE`; mixing naive and tz-aware in UNION/CASE must emit `TypeMismatch`; `DATE_TRUNC` must preserve the tz-axis of its input — all three enforced by explicit regression tests.

- **Standing decimal gates.** `cargo test -p smelt-db --test nullability_property_tests` must stay green (no change from §11 gate). The decimal arithmetic growth formulas and division rejection must be covered by the type property oracle (`cargo test -p smelt-db --test type_property_tests`) after the decimal plan lands.

## Known Divergences / Open Questions

- **`AT TIME ZONE` conversion operator not yet implemented.** The SQL expression `expr AT TIME ZONE zone_str` converts between naive and tz-aware timestamps: `Timestamp AT TIME ZONE 'zone' → Timestamp WITH TIME ZONE`; `Timestamp WITH TIME ZONE AT TIME ZONE 'zone' → Timestamp`. The operator is supported by DuckDB and PostgreSQL; Spark uses different functions (`to_utc_timestamp`, `from_utc_timestamp`) so a portable mapping requires a design pass. The smelt parser has no AST node for `AT TIME ZONE`; the expression is currently parsed as an unrecognised construct and infers `Unknown`. Until it lands, the escape hatches are an explicit `CAST` or the `TIMEZONE(zone, expr)` scalar function (not yet registered; available on DuckDB and Spark with differing argument semantics).

- **Promotion chain implementation drift.** `crates/smelt-db/src/type_inference.rs::promote_types` orders the chain `SmallInt < Integer < BigInt < Float < Decimal < Double`. `docs/type_semantics.md` documents `Float < Decimal < Double`. The normative chain in this spec is the research-aligned one (research §9): `SmallInt < Integer < BigInt < Decimal < Double`, `Float` collapsed into `Double`. The `Float < Decimal` ordering divergence in `promote_types` and the `Float` enum variant are follow-up work. (`numeric_lub` in `smelt-types` now correctly applies the §15 LUB formula for Decimal pairs and integer-lifting.)
- **Engine-bound decimal division is not yet available.** Division with a Decimal operand — `Decimal / T` and `T / Decimal` alike — is rejected in portable code (§15); an integer-family numerator over a Decimal denominator does not coerce to a `Decimal` result. The `Float`/`Double` counterpart (`Float / Decimal`, `Double / Decimal`) is the carve-out: it promotes to a portable floating result and is allowed. The escape hatch of declaring an engine on a model to get native division semantics (`Double` on DuckDB, `Decimal` on Spark, `NUMERIC` on Postgres) is deferred to the engine-declaration feature. Until then, users must cast to `Double` or `Float` explicitly. Tracked in `docs/research/20260516-decimal-type-system.md` §7.
- **ByDesign aggregate divergences.** `SUM(Decimal(p, s))` returns `Decimal(38, s)` where Spark returns `Decimal(min(p+10, 38), s)` — smelt's result is conservative (wider precision). `AVG(Decimal)` returns `Double` where Spark returns `Decimal` — smelt's result is the portable choice agreed with DuckDB. Both are registered ByDesign in the divergence registry (`crates/smelt-db/tests/prop_helpers/divergences.rs`).
- **`SIGN(Decimal)` returns `SmallInt`.** DuckDB returns `TINYINT` (same), Spark returns `Decimal`; ByDesign divergence from Spark, registered in the divergence registry.
- **Nullability is not yet folded into the output fingerprint.** `output_fingerprint.md` treats nullability as breaking-by-default (conservative rebuild). Folding the tracked axis into the fingerprint is deferred until the fingerprint is wired into the runtime (see `docs/ROADMAP.md` item 5); the soundness contract here is the precondition for that fold. The fold must hash the structured `TypedColumn` (type + nullability), never a rendered display string, so display conventions can evolve without invalidating fingerprints.
- **Nullability precision is deliberately coarse.** `WHERE x IS NOT NULL` narrowing, join-key non-null reasoning, nested composite tracking (struct fields, array elements), and other flow-sensitive refinements are out of scope; the contract permits them later as pure precision improvements (flipping `nullable: true → false` where provable) without a contract change. Nested composite tracking, if ever pursued, extends the composite `DataType` variants (see Design §"Axis placement").
- **Fragment sort coverage.** `Expr<T>`, `TableExpr`, and `TableExpr<{…}>` are landed. `AggExpr<T>` and `WindowExpr<T>` are partially landed: the `ExprKind` axis enforces the kind ceiling at splice points (`WindowInScalarContext`), but type-annotation parsing for `AggExpr<T>` / `WindowExpr<T>` may still be in flight (tracked in `docs/plans/20260422-smelt-functions.md`). Validate against the live `crates/smelt-types/src/signatures.rs::SmeltType` enum.
- **`Float` as a distinct DataType.** `DataType::Float` exists in code; research treats Float as Double. This spec aligns with research and lists `Float` collapsing into `Double` as the normative rule. `Float` may be removed from the enum in a future plan.
- **`docs/type_semantics.md` overlap.** The legacy quasi-spec contains backend-divergence material that is still useful (DuckDB/Spark divergence registry). Recommendation: keep it as a backend-divergence appendix referenced from this spec; over time, fold or trim.
- **`Map<K,V>` rules.** `DataType::Map` exists in the vocabulary but research is silent on its semantics. This spec marks `Map` as non-`Ordered`; broader rules for Map equality, ordering, and arithmetic remain open.
- **Silent `Unknown`s exist today; the reason-discriminant and `ColumnTypeUnresolved` are not yet enforced.** The `Unknown` value is currently undiscriminated, and several inference gaps produce `Unknown` columns with no diagnostic — violating the no-silent-`Unknown` invariant: generator-emitted and `smelt.columns_of`-reflected model schemas, and meta-language HOF values placed in SQL column position (`meta_language.md`). Function-derived columns (`smelt.functions.*` struct-spread and `TableExpr` results) now resolve — see `function_schema_inference.md` — so they are no longer a silent-`Unknown` source. `(VALUES …) AS t(c₁, …)` derived-table columns now also resolve to concrete types. Cross-family binary arithmetic (`42 + '3'`, `TRUE + 1`) is also enforced: the result degrades to `Unknown` and a `TypeMismatch` Error is emitted at the operator span — it is not a silent-`Unknown` source. The composite-path gaps (array-literal element unification and `UNION` branch unification with mixed families) still degrade to `CannotInferType` without a `TypeMismatch`. What remains is the meta-language surface. Enforcement is sequenced after those gaps close, so the diagnostic flags genuine defects rather than known-deferred ones. Tracked in `docs/plans/20260519-functions-meta-gaps.md`.
- **Diagnostic codes pre-`diagnostics.md`.** Codes listed in this spec are owned here until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)

## References

### Code

- `crates/smelt-types/src/lib.rs` — `DataType`, `TypedColumn`, `is_numeric` / `is_string` / `is_temporal`, `normalize`, `to_backend_sql`
- `crates/smelt-types/src/parse.rs` — type-string parsing (sources YAML, `CAST`, type annotations)
- `crates/smelt-types/src/signatures.rs` — `SmeltType`, `ExprKind`, `TypeConstraint`, `SchemaRequirement`, `RowTail`, `StructRowTail`, `BuiltinRegistry`, `unify_call`, `numeric_lub`, `kind_ceiling`, `subkind_of`
- `crates/smelt-types/src/functions.rs` — `SqlFunction`, `FunctionCategory`
- `crates/smelt-db/src/type_inference.rs` — pure inference (`TypeContext`, `infer_expression_type`, `infer_expression_kind`, `promote_types`, `infer_select_column_types`, `check_window_in_scalar_contexts`)
- `crates/smelt-db/src/schema.rs` — `ModelSchema`, `Column`, `ColumnSource`, `RowExtension`, `InputConstraint`, `ModelFunctionType`
- `crates/smelt-db/src/function_body_check.rs` — Tier 1 / 2 / 3 body checking
- `crates/smelt-db/src/lib.rs::DiagnosticCode` — diagnostic surface

### Tests

- `crates/smelt-db/tests/type_property_tests.rs` — DuckDB oracle for type inference (type correctness, not nullability)
- `crates/smelt-db/tests/nullability_property_tests.rs` — value-based DuckDB oracle for nullability soundness (§11 verification gate: single-table, two-table joins, and set-operation coverage)
- `crates/smelt-types/tests/registry_coverage.rs` — built-in registry coverage
- Unit tests under `crates/smelt-db/src/type_inference.rs::tests` and `function_body_check.rs::tests`

### Plans (history) — oldest → newest

- `docs/plans/20260321-dialect-function-remapping.md`
- `docs/plans/20260404-parser-type-testing-completeness.md`
- `docs/plans/20260405-schema-evolution-complex-types.md`
- `docs/plans/20260422-smelt-functions.md`
- `docs/plans/20260610-nullability-soundness.md`
- `docs/plans/20260611-decimal-arithmetic.md`

### Related specs

- `docs/specs/architecture.md` — system-level pipeline; this spec sits inside its Analyze stage.
- `docs/specs/incremental_models.md` — downstream consumer of `ModelSchema`.
- `docs/specs/meta_language.md` — `List<T>` fragment-sort surface and semantics; `ModelDef` field rules, generator-file body semantics, and construction restrictions; this spec registers the type vocabulary entries, the meta-language spec owns the rules.
- `docs/specs/function_schema_inference.md` — how `smelt.functions.*` calls contribute columns/types to a caller's schema; owns the `ColumnTypeUnresolved` schema-propagation rules; this spec owns the `Unknown` reason-discriminant and the no-silent-`Unknown` doctrine it consumes.

### Backend divergence appendix

`docs/type_semantics.md` documents intentional smelt-vs-backend choices (truncating int division, `BigInt` vs `Decimal(38,0)` for `SUM`, `Text` vs `Varchar`) and the divergence registry at `crates/smelt-db/tests/prop_helpers/divergences.rs`. Treat that document as the backend-divergence appendix to this spec; `docs/specs/types.md` is the canonical source for normative type rules.
