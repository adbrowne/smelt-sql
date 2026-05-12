# Meta-Language Reference

Alphabetical quick reference for all meta-language constructs and diagnostic codes. Covers list literals, the spread operator, every HOF, reducer, lambda keyword, the pipe operator, `smelt.config.var`, and the reflection surface (`smelt.columns_of`, `ColumnRef`, identifier lift, wide reflection accessors `smelt.models.*` / `smelt.sources.*`, `ModelRef`, `SourceRef`).

For a conceptual introduction, see [Overview](index.md). For detailed explanations, see the per-construct pages: [Lists & Spread](lists.md), [Lambdas](lambdas.md), [Higher-Order Functions](hofs.md), [Pipe Operator](pipes.md), [Reducers](reducers.md), [Config Variables](config-vars.md), [Reflection](reflection.md).

---

## `and_all` — Boolean AND reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Boolean>>, and_all) -> Expr<Boolean>
```

**Empty-list identity:** `TRUE`

**Example:**
```sql
-- reduce([is_active, age > 18], and_all) → is_active AND age > 18
SELECT id FROM smelt.sources.raw.users
WHERE reduce([is_active, age > 18], and_all)
```

See [Reducers — `and_all`](reducers.md#and_all) for full details.

---

## `ColumnRef` — closed meta record type for column reflection

**Kind:** closed meta-only record type; produced exclusively by `smelt.columns_of`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `name` | `Text` | Column identifier (un-quoted; case-preserved) |
| `type` | `DataType` | Column's `DataType` |
| `is_numeric` | `Boolean` | `TRUE` iff `type` is in the `Numeric` constraint set |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `ColumnRefFieldUnknown`. `ColumnRef` is not user-constructible — values originate only from `smelt.columns_of`.

**Example:**
```sql
smelt.columns_of(smelt.orders)
  |> filter(fn c => c.is_numeric)   -- c.is_numeric : Boolean
  |> map(fn c => c.name)            -- c.name : Text, lifts to identifier in splice
```

**Editor support:** hover on a `ColumnRef`-typed binding shows `ColumnRef` and the closed field list with each field's type; completion at `c.<cursor>` offers `name`, `type`, `is_numeric`.

See [Reflection — `ColumnRef`](reflection.md#columnref) for the closed-field contract, body-check vs expansion-time behaviour, and diagnostic codes.

---

## `comma_sep` — comma-separated select-items reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<T>>, comma_sep) -> SelectItems<Scalar>
```

**Empty-list identity:** empty `SelectItems` (adjacent commas elide at splice)

**Example:**
```sql
SELECT reduce([id, name, email], comma_sep) FROM smelt.sources.raw.users
-- Engine sees: SELECT id, name, email FROM ...
```

See [Reducers — `comma_sep`](reducers.md#comma_sep) for full details.

---

## `concat` — text concatenation reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Text>>, concat) -> Expr<Text>
```

**Empty-list identity:** `''` (empty string)

**Example:**
```sql
SELECT reduce(['hello', ' ', 'world'], concat)
-- Engine sees: SELECT 'hello' || ' ' || 'world'
```

See [Reducers — `concat`](reducers.md#concat) for full details.

---

## `filter` — HOF: keep list elements matching a predicate

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
filter(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>
```

**Example:**
```sql
-- Keep only positive numbers
SELECT filter([1, -2, 3], fn c => c > 0)
```

**Editor support:** hover shows `List<T>` with `T` from the input element type.

See [Higher-Order Functions — `filter`](hofs.md#filter) for full details and diagnostic codes.

---

## `fn` — lambda keyword

**Kind:** reserved keyword; introduces a lambda expression.

**Syntax:**
```
fn IDENT => EXPR
```

Only valid as a positional argument to a HOF (`map`, `filter`). A lambda cannot be assigned to a name, stored in a list, or passed as a named argument.

**Example:**
```sql
-- fn c => c * 2 doubles each element of the list
SELECT map([1, 2, 3], fn c => c * 2)
```

**Editor support:** hover on the parameter inside the body shows its bound type; goto-definition resolves to the `fn` binding occurrence.

See [Lambdas](lambdas.md) for full details, scoping rules, and diagnostic codes.

---

## `intersect_all` — table INTERSECT ALL reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<TableExpr>, intersect_all) -> TableExpr
```

**Empty-list identity:** none — `ReducerEmptyNoIdentity` on an empty list.

**Example:**
```sql
SELECT * FROM reduce(
    [smelt.ref('active_users'), smelt.ref('premium_users')],
    intersect_all
)
```

See [Reducers — `intersect_all`](reducers.md#intersect_all) for full details.

---

## `List<T>` — meta list type

**Kind:** meta-only type; never appears in data-world SQL.

**Definition:** a finite, ordered, immutable sequence of elements of type `T`. Length is fixed at construction. `T` is a fragment sort (`Expr<U>`, `OrderSpec`, …) or a data-type lifted as a meta literal (`Text`, `Integer`, …).

**Covariance:** `List<S> <: List<T>` whenever `S <: T`. Sound because lists are immutable.

**Construction:** list literals `[a, b, c]`; HOFs `map`, `filter`.

**Hover:** hovering over a list literal shows `List<T>` with `T` resolved to the inferred element type, e.g. `List<Expr<INTEGER>>`.

**Example:**
```sql
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
--     ^^^^^^^^^^
--     List<Expr<INTEGER>> — hover shows this type in the editor
```

See [Lists & Spread — The `List<T>` type](lists.md#the-listt-type) for the covariance rule and subtyping details.

---

## `map` — HOF: apply a lambda to every element

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
map(xs: List<T>, f: Lambda<T, U>) -> List<U>
```

**Example:**
```sql
-- Double every element
SELECT map([1, 2, 3], fn c => c * 2)
```

**Editor support:** hover shows `List<U>` with `U` from the lambda body's inferred type.

See [Higher-Order Functions — `map`](hofs.md#map) for full details and diagnostic codes.

---

## Meta-`Text`-as-identifier lift positions {#meta-text-as-identifier-lift-positions}

When a meta-`Text` value is spliced into a position where the SQL grammar expects an unquoted **identifier**, smelt lifts that value to the identifier. The lift applies in exactly four positions:

| Position | Example |
|---|---|
| Column-reference position inside an expression | `COALESCE(c.name, 0)` — `c.name` lifts to a column identifier |
| `AS` alias of a SELECT item | `SUM(amount) AS c.name` — `c.name` lifts to the output alias |
| `ORDER BY` column reference | `ORDER BY c.name` — `c.name` lifts to a sort key |
| `GROUP BY` column reference | `GROUP BY c.name` — `c.name` lifts to a grouping key |

In all other positions a meta-`Text` retains its string-value meaning. The lifted identifier is validated against the surrounding splice context using the standard scoping rule; an unrecognised column name emits `UnknownColumn`.

The lift applies **only to compile-time meta-`Text` values**, not to runtime `Expr<Text>` values.

See [Reflection — Meta-`Text`-as-identifier lift](reflection.md#meta-text-as-identifier-lift) for full details and examples.

---

## `ModelRef` — closed meta record type for model reflection

**Kind:** closed meta-only record type; produced by `smelt.models.with_tag` and `smelt.models.all`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative path (e.g. `models/orders.sql`) |
| `name` | `Text` | Short model name (stem, e.g. `orders`) |
| `tags` | `List<Text>` | Merged tag set (smelt.yml first, then frontmatter) |
| `columns` | `List<ColumnRef>` | Column list — equivalent to `smelt.columns_of(m)` |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `ModelRefFieldUnknown`. `ModelRef` is not user-constructible — values originate only from `smelt.models.*` accessors.

**Subtyping:** `ModelRef <: TableExpr`. Pass a `ModelRef` anywhere a `TableExpr` is required (e.g. `smelt.columns_of`, `reduce(..., union_all)`) without explicit projection.

**Example:**
```sql
-- Collect the name of every model tagged 'cohort'
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)

-- m.columns is equivalent to smelt.columns_of(m)
SELECT map(smelt.models.with_tag('cohort'), fn m => m.columns)
```

**Editor support:** hover on a `ModelRef`-typed binding shows `ModelRef` and the closed field list with each field's type; completion at `m.<cursor>` offers `path`, `name`, `tags`, `columns`.

See [Reflection — `ModelRef`](reflection.md#modelref) for the closed-field contract, subtyping rules, and diagnostic codes.

---

## `or_any` — Boolean OR reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Boolean>>, or_any) -> Expr<Boolean>
```

**Empty-list identity:** `FALSE`

**Example:**
```sql
-- reduce([is_admin, is_moderator], or_any) → is_admin OR is_moderator
SELECT id FROM smelt.sources.raw.users
WHERE reduce([is_admin, is_moderator], or_any)
```

See [Reducers — `or_any`](reducers.md#or_any) for full details.

---

## `plus_chain` — numeric addition reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<Expr<Numeric>>, plus_chain) -> Expr<Numeric>
```

**Empty-list identity:** `0` cast to the LUB element type.

**Example:**
```sql
SELECT reduce([1, 2, 3], plus_chain)
-- Engine sees: SELECT 1 + 2 + 3
```

See [Reducers — `plus_chain`](reducers.md#plus_chain) for full details.

---

## `reduce` — HOF: fold a list into a single fragment

**Kind:** built-in higher-order function; reserved name.

**Signature:**
```
reduce(xs: List<T>, r) -> OutputSort
```

where `r` is a bare reducer identifier from the closed registry and `OutputSort` is the reducer's declared output sort.

**Example:**
```sql
SELECT reduce([true, false, true], and_all)
-- Engine sees: SELECT true AND false AND true
```

**Editor support:** hover on the reducer name shows its input element type, output sort, and empty-list identity.

See [Higher-Order Functions — `reduce`](hofs.md#reduce) and [Reducers](reducers.md) for full details.

---

## `smelt.columns_of` — compile-time column list accessor

**Kind:** built-in meta-only accessor; returns `List<ColumnRef>`.

**Signature:**
```
smelt.columns_of(t: TableExpr) -> List<ColumnRef>
```

Returns the column list of a `TableExpr`-valued argument as a `List<ColumnRef>`, preserving declared column order. Must be called with exactly one positional argument; named arguments emit `ColumnsOfNamedArgument`. The argument must be `TableExpr`-typed; mismatches emit `ColumnsOfRequiresTableExpr`.

**Example:**
```sql
smelt.columns_of(smelt.orders)
  |> filter(fn c => c.is_numeric)
  |> map(fn c => COALESCE(c.name, 0))
```

**Editor support:** hover on `smelt.columns_of(t)` shows `List<ColumnRef>` and, when `t`'s schema is statically resolvable, the resolved column count plus the first five column names; completion at the argument position offers in-scope `TableExpr`-valued names.

See [Reflection](reflection.md) for the full surface, body-check vs expansion-time semantics, and worked example.

---

## `smelt.config.var` — compile-time variable lookup

**Kind:** built-in compile-time function; returns `Text`.

**Signature:**
```
smelt.config.var(name: Text) -> Text
```

Reads `name` from the `vars:` block of `smelt.yml`. The argument must be a string literal.

**Example:**
```sql
SELECT smelt.config.var('region')
-- Resolves to: SELECT 'us-west-2'  (when smelt.yml declares vars: {region: us-west-2})
```

**Editor support:** hover shows `Text` and the variable's resolved value; goto-definition resolves to the `vars.name:` line in `smelt.yml`.

See [Config Variables](config-vars.md) for YAML scalar coercion rules, diagnostic codes, and worked examples.

---

## `smelt.models.all` — all workspace models

**Kind:** compile-time workspace accessor; returns `List<ModelRef>`.

**Signature:**
```
smelt.models.all() -> List<ModelRef>
```

Returns every model in the workspace, sorted by workspace-relative path (byte-lexicographic, `/` separator). Takes no arguments; any argument emits `WideReflectionUnexpectedArgument`.

**Example:**
```sql
-- All model paths in the workspace
SELECT map(smelt.models.all(), fn m => m.path)
```

**Editor support:** hover on `smelt.models.all()` shows `List<ModelRef>` and the resolved model count; completion offers `smelt.models.all` in `smelt.models.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.models.with_tag` — workspace models filtered by tag

**Kind:** compile-time workspace accessor; returns `List<ModelRef>`.

**Signature:**
```
smelt.models.with_tag(tag: Text) -> List<ModelRef>
```

Returns all workspace models whose tag set contains `tag`, sorted by workspace-relative path. Argument must be a single positional compile-time string literal. Named arguments emit `WithTagNamedArgument`; non-literal arguments emit `WithTagRequiresText`.

**Example:**
```sql
-- All models tagged 'cohort', sorted by path
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)
```

**Editor support:** hover on `smelt.models.with_tag(...)` shows `List<ModelRef>` and the resolved model count for the given tag; completion offers `smelt.models.with_tag` in `smelt.models.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.sources.all` — all workspace sources

**Kind:** compile-time workspace accessor; returns `List<SourceRef>`.

**Signature:**
```
smelt.sources.all() -> List<SourceRef>
```

Returns every declared source in the workspace, sorted by workspace-relative path (byte-lexicographic, `/` separator). Takes no arguments; any argument emits `WideReflectionUnexpectedArgument`.

**Example:**
```sql
-- All source paths in the workspace
SELECT map(smelt.sources.all(), fn s => s.path)
```

**Editor support:** hover on `smelt.sources.all()` shows `List<SourceRef>` and the resolved source count; completion offers `smelt.sources.all` in `smelt.sources.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `smelt.sources.with_tag` — workspace sources filtered by tag

**Kind:** compile-time workspace accessor; returns `List<SourceRef>`.

**Signature:**
```
smelt.sources.with_tag(tag: Text) -> List<SourceRef>
```

Returns all workspace sources whose tag set contains `tag`, sorted by workspace-relative path. Argument must be a single positional compile-time string literal. Named arguments emit `WithTagNamedArgument`; non-literal arguments emit `WithTagRequiresText`.

**Example:**
```sql
-- All sources tagged 'audit'
SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)
```

**Editor support:** hover on `smelt.sources.with_tag(...)` shows `List<SourceRef>` and the resolved source count for the given tag; completion offers `smelt.sources.with_tag` in `smelt.sources.<cursor>` context.

See [Reflection — Wide reflection](reflection.md#wide-reflection-workspace-introspection) for full details, argument rules, and diagnostic codes.

---

## `SourceRef` — closed meta record type for source reflection

**Kind:** closed meta-only record type; produced by `smelt.sources.with_tag` and `smelt.sources.all`.

**Fields:**

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative path (e.g. `models/sources/raw/orders.yml`) |
| `name` | `Text` | Short source name (e.g. `orders`) |
| `tags` | `List<Text>` | Merged tag set (smelt.yml first, then frontmatter) |
| `columns` | `List<ColumnRef>` | Column list — equivalent to `smelt.columns_of(s)` |

Access fields with dot-notation inside a HOF lambda. Any other field name emits `SourceRefFieldUnknown`. `SourceRef` is not user-constructible — values originate only from `smelt.sources.*` accessors.

**Subtyping:** `SourceRef <: TableExpr`. Pass a `SourceRef` anywhere a `TableExpr` is required without explicit projection.

**Example:**
```sql
-- Collect the name of every source tagged 'audit'
SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)
```

**Editor support:** hover on a `SourceRef`-typed binding shows `SourceRef` and the closed field list with each field's type; completion at `s.<cursor>` offers `path`, `name`, `tags`, `columns`.

See [Reflection — `SourceRef`](reflection.md#sourceref) for the closed-field contract, subtyping rules, and diagnostic codes.

---

## `union_all` — table UNION ALL reducer

**Kind:** contextual reducer (closed registry); use as the second argument to `reduce`.

**Signature:**
```
reduce(xs: List<TableExpr>, union_all) -> TableExpr
```

**Empty-list identity:** none — `ReducerEmptyNoIdentity` on an empty list.

**Example:**
```sql
SELECT * FROM reduce(
    [smelt.ref('orders_2024'), smelt.ref('orders_2025')],
    union_all
)
```

See [Reducers — `union_all`](reducers.md#union_all) for full details.

---

## `...xs` — spread operator

**Type:** consumes `List<T>`; materialises `T` elements into the surrounding comma-separated grammar position.

**Syntax:**
```
...expr
```

where `expr` evaluates to a `List<T>`.

**Valid positions:** SELECT lists.

**Planned but not yet supported:** GROUP BY, ORDER BY, positional function arguments, IN-lists, VALUES rows, inside other list literals.

**Forbidden positions:** WHERE clauses, FROM clauses without an explicit reducer, boolean-composition contexts (`AND`/`OR`), named-argument positions (`name => value`). Each forbidden use emits `MetaSpreadInForbiddenPosition`.

**Empty-list behaviour:** `...[]` elides itself and adjacent commas silently.

**Hover:** hovering over `...xs` in the editor shows the type of the source list, e.g. `List<Expr<INTEGER>>`.

**Example:**
```sql
-- Spread two column references into SELECT
SELECT id, ...[name, email] FROM smelt.sources.raw.users
-- Engine sees: SELECT id, name, email FROM ...
```

See [Lists & Spread — Spread operator](lists.md#spread-operator-xs) for full details and forbidden-position examples.

---

## `[…]` — list literal

**Type:** `List<T>` where `T` is the LUB of element types.

**Syntax:**
```
[ expr, expr, … ]          -- one or more elements
[ expr, expr, …, ]         -- trailing comma allowed
[ expr ]                   -- singleton
[]                         -- empty (requires inferable target sort)
```

**Disambiguation:** the same `[…]` token sequence may resolve to a meta `List<T>` or a data-world `Array<U>` depending on the surrounding context. When both readings are valid, meta wins.

**Example:**
```sql
-- List<Expr<INTEGER>> spliced into a SELECT list
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
-- Engine sees: SELECT 1, 2, 3 FROM ...
```

See [Lists & Spread — List literal syntax](lists.md#list-literal-syntax-a-b-c) for full details.

---

## `|>` — pipe operator

**Kind:** meta-world binary operator; purely syntactic sugar desugared before type-checking.

**Semantics:**
```
LHS |> f(args...)   ≡   f(LHS, args...)
```

Left-associative, lowest meta-language precedence. RHS must be a call expression.

**Example:**
```sql
-- examples/meta_hofs/models/pipe_rewrite.sql
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- desugars to: map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
```

**Editor support:** hover shows the result type of the equivalent un-piped call.

See [Pipe Operator](pipes.md) for full details and diagnostic codes.

---

## Diagnostic codes

Alphabetical across the whole meta-language surface.

### `ColumnRefFieldUnknown`

**When:** Field access on a `ColumnRef` value uses an identifier that is not one of the three declared fields (`name`, `type`, `is_numeric`).

**Message:** `ColumnRef has no field {name}; expected one of: name, type, is_numeric`

**Fix:** use `c.name` (Text), `c.type` (DataType), or `c.is_numeric` (Boolean). Any other field requires a spec extension.

See [Reflection — `ColumnRefFieldUnknown`](reflection.md#columnrefieldunknown).

---

### `ColumnsOfNamedArgument`

**When:** `smelt.columns_of` is called with a named argument instead of a positional one.

**Message:** `smelt.columns_of takes one positional argument; named arguments are not supported`

**Fix:** pass the `TableExpr` positionally: `smelt.columns_of(smelt.orders)`.

See [Reflection — `ColumnsOfNamedArgument`](reflection.md#columnsofnamedargument).

---

### `ColumnsOfRequiresTableExpr`

**When:** The argument to `smelt.columns_of` synthesises to a type not assignable to `TableExpr`.

**Message:** `smelt.columns_of expects TableExpr; found {actual}`

**Fix:** pass a `smelt.<path>` reference to a model, source, or seed, or a `TableExpr` parameter of the enclosing `smelt.define` function.

See [Reflection — `ColumnsOfRequiresTableExpr`](reflection.md#columnsofRequirestableexpr).

---

### `ColumnsOfUnresolvableSchema`

**When:** At expansion time, `smelt.columns_of(t)` cannot resolve the schema for `t` (for example because an upstream model has an unknown schema).

**Message:** `cannot resolve column list for {t}; upstream schema is unknown`

**Fix:** ensure the upstream model, source, or seed has a fully declared schema and compiles cleanly. This diagnostic suppresses further errors from the surrounding HOF call.

See [Reflection — `ColumnsOfUnresolvableSchema`](reflection.md#columnsofunresolvableschema).

---

### `ConfigVarNameNotLiteral`

**When:** The argument to `smelt.config.var` is not a string literal.

**Message:** `smelt.config.var name must be a string literal`

**Fix:** use a string literal: `smelt.config.var('my_var')`. Dynamic name resolution is planned but not yet implemented.

See [Config Variables — `ConfigVarNameNotLiteral`](config-vars.md#configvarnamenotliteral).

---

### `ConfigVarNotFound`

**When:** `smelt.config.var('name')` is called but `name` is not declared in `smelt.yml` `vars:`.

**Message:** `compile-time variable {name} not declared in smelt.yml vars`

**Fix:** add `name:` under `vars:` in `smelt.yml`, or check for typos in the variable name.

See [Config Variables — `ConfigVarNotFound`](config-vars.md#configvarnotfound).

---

### `ConfigVarNullCoercion` (warning)

**When:** A `vars:` entry has a YAML `null` value, coerced to `''` at the call site.

**Message:** `null variable {name} coerced to empty string; declare a default in smelt.yml`

**Fix:** replace the null YAML value with an explicit default string.

See [Config Variables — `ConfigVarNullCoercion`](config-vars.md#configvarnullcoercion-warning).

---

### `HofExpectsLambda`

**When:** The second argument to `map` or `filter` is not a lambda.

**Message:** `{hof} expects a lambda; found {actual type}`

**Fix:** replace the second argument with a `fn x => body` lambda.

See [Higher-Order Functions — `HofExpectsLambda`](hofs.md#hofexpectslambda).

---

### `HofExpectsReducer`

**When:** The second argument to `reduce` is not a bare reducer identifier from the closed registry.

**Message:** `reduce expects a reducer; found {actual}`

**Fix:** use one of the seven registered reducer names. See [Reducers](reducers.md) for the full list.

See [Higher-Order Functions — `HofExpectsReducer`](hofs.md#hofexpectsreducer).

---

### `HofNameShadowed`

**When:** A `smelt.define` function is declared with the name `map`, `filter`, or `reduce`.

**Message:** `{name} is a reserved higher-order function name`

**Fix:** rename the `smelt.define` function.

See [Higher-Order Functions — `HofNameShadowed`](hofs.md#hofnameshadowed).

---

### `LambdaArityNotSupported`

**When:** A lambda with more than one parameter is written: `fn (a, b) => body`.

**Message:** `multi-argument lambdas are not supported in v1; use a single parameter`

**Fix:** rewrite to use a single parameter. Multi-argument lambdas are planned but not yet implemented.

See [Lambdas — `LambdaArityNotSupported`](lambdas.md#lambdaaritynotsupported).

---

### `LambdaInForbiddenPosition`

**When:** A `fn x => body` lambda appears outside a HOF positional argument position.

**Message:** `lambda is only valid as an argument to a higher-order function`

**Fix:** move the lambda inside a `map` or `filter` call.

See [Lambdas — `LambdaInForbiddenPosition`](lambdas.md#lambdainforbiddenposition).

---

### `LambdaResultTypeMismatch`

**When:** The lambda body's type is incompatible with what the surrounding HOF requires (e.g. `filter` requires `Boolean`).

**Message:** `{hof} requires lambda result {expected}; found {actual}`

**Fix:** adjust the body expression to produce the required type.

See [Lambdas — `LambdaResultTypeMismatch`](lambdas.md#lambdaresulttypemismatch).

---

### `MetaListEmptyTypeUnknown`

**When:** A bare `[]` literal appears where the type checker cannot infer the element type from context.

**Message:** `cannot infer element type for empty list literal`

**Fires at:** the `[]` CST span.

**Example:**
```sql
SELECT
    id,
    []   -- MetaListEmptyTypeUnknown
FROM smelt.sources.raw.users
```

**Fix:** provide elements so the type can be inferred; use `...[]` (spread of an empty list) for silent elision; or annotate a `smelt.define` parameter with the expected `List<T>` type so the empty literal has a target sort.

---

### `MetaListHeterogeneous`

**When:** The elements of a list literal do not share a common type under LUB.

**Message:** `list elements have incompatible types: {T0}, {Tk}`

**Fires at:** the offending list literal CST span.

**Example:**
```sql
SELECT id, ...[1, 'hello'] FROM smelt.sources.raw.users
--              ^^^^^^^^^^^  MetaListHeterogeneous: INTEGER vs TEXT
```

**Fix:** ensure all elements share a compatible type. Numeric mixed precision is promoted automatically (`[1, 2.5]` infers `DECIMAL`). For truly incompatible types, cast all elements to a common type or split into separate lists.

---

### `MetaSpreadInForbiddenPosition`

**When:** A `...xs` spread operator appears in a grammar position that does not support spread. Forbidden positions: WHERE clause, FROM clause without an explicit reducer, boolean-composition context (`AND`/`OR`), named-argument position.

**Message:** `spread is not allowed in {position name}`

**Fires at:** the `...` CST span.

**Example:**
```sql
SELECT id
FROM smelt.sources.raw.users
WHERE id = 1 AND ...preds  -- MetaSpreadInForbiddenPosition
```

**Fix:** move the spread to a SELECT list. For WHERE-clause predicate lists, use the `and_all` reducer. For IN-list membership, use `WHERE id IN (...vs)` (planned but not yet implemented).

!!! note
    Forbidden positions other than WHERE may currently emit parse errors rather than this diagnostic. The full set of friendly diagnostics for forbidden positions is planned but not yet wired everywhere.

---

### `MetaSpreadOnNonList`

**When:** The `...` operator is applied to an expression that does not have type `List<T>`.

**Message:** `spread expects List<T>; found {actual type}`

**Fires at:** the `...` CST span.

**Example:**
```sql
SELECT id, ...some_integer FROM smelt.sources.raw.users
--         ^^^^^^^^^^^^^^  MetaSpreadOnNonList: INTEGER is not List<T>
```

**Fix:** wrap the value in a list literal (`...[some_integer]`) to splice a single element, or verify that the binding supplying the value actually has type `List<T>`.

---

### `ModelRefFieldUnknown`

**When:** Field access on a `ModelRef` value uses an identifier that is not one of the four declared fields (`path`, `name`, `tags`, `columns`).

**Message:** `ModelRef has no field {name}; expected one of: path, name, tags, columns`

**Fix:** use `m.path` (Text), `m.name` (Text), `m.tags` (List\<Text\>), or `m.columns` (List\<ColumnRef\>). Any other field requires a spec extension.

See [Reflection — `ModelRefFieldUnknown`](reflection.md#modelreffieldunknown).

---

### `PipeInDataPosition`

**When:** A `|>` pipe expression appears in a Data-World grammar position.

**Message:** `|> is meta-only; use SQL composition in this position`

**Fix:** move the pipe chain to a meta-world context, or use SQL operators directly in the data position.

See [Pipe Operator — `PipeInDataPosition`](pipes.md#pipeindataposition).

---

### `PipeRhsNotCall`

**When:** The right-hand side of `|>` is not a function call expression.

**Message:** `pipe right-hand side must be a function call`

**Fix:** write the RHS as a call: `LHS |> f(args)`.

See [Pipe Operator — `PipeRhsNotCall`](pipes.md#piperhsnotcall).

---

### `ReducerEmptyNoIdentity`

**When:** `reduce` is called with an empty list using `union_all` or `intersect_all`, which have no identity element.

**Message:** `reducer {r} has no identity for an empty list`

**Fix:** ensure the source list is non-empty, or use a reducer that has an empty-list identity.

See [Reducers — `ReducerEmptyNoIdentity`](reducers.md#reduceremptynoidentity).

---

### `ReducerInputTypeMismatch`

**When:** `reduce` is called with a list whose element type is incompatible with the reducer's declared input.

**Message:** `reducer {r} expects List<{T_in}>; found List<{T_actual}>`

**Fix:** use `map` to convert the list elements to the correct type first, or choose a different reducer.

See [Reducers — `ReducerInputTypeMismatch`](reducers.md#reducerinputtypemismatch).

---

### `ReducerNameShadowed`

**When:** A `smelt.define` function is declared with a name that matches one of the seven reserved reducer names.

**Message:** `{name} is a reserved reducer name`

**Fix:** rename the `smelt.define` function.

See [Reducers — `ReducerNameShadowed`](reducers.md#reducernameshadowed).

---

### `SourceRefFieldUnknown`

**When:** Field access on a `SourceRef` value uses an identifier that is not one of the four declared fields (`path`, `name`, `tags`, `columns`).

**Message:** `SourceRef has no field {name}; expected one of: path, name, tags, columns`

**Fix:** use `s.path` (Text), `s.name` (Text), `s.tags` (List\<Text\>), or `s.columns` (List\<ColumnRef\>). Any other field requires a spec extension.

See [Reflection — `SourceRefFieldUnknown`](reflection.md#sourcereffieldunknown).

---

### `WithTagNamedArgument`

**When:** `smelt.models.with_tag` or `smelt.sources.with_tag` is called with a named argument instead of a positional one.

**Message:** `with_tag takes one positional Text literal; named arguments are not supported`

**Fix:** use positional syntax: `with_tag('my-tag')` not `with_tag(tag => 'my-tag')`.

See [Reflection — `WithTagNamedArgument`](reflection.md#withtagnamedargument).

---

### `WithTagRequiresText`

**When:** The argument to `smelt.models.with_tag` or `smelt.sources.with_tag` is not a compile-time string literal (e.g. it is an integer or a runtime expression like `UPPER('cohort')`).

**Message:** `with_tag requires a compile-time string literal; found {actual}`

**Fix:** pass a string literal: `with_tag('my-tag')`. Dynamic tag filtering is not supported.

See [Reflection — `WithTagRequiresText`](reflection.md#withtagrequirestext).

---

### `WideReflectionUnexpectedArgument`

**When:** `smelt.models.all` or `smelt.sources.all` is called with one or more arguments.

**Message:** `all() takes no arguments; found {n} argument(s)`

**Fix:** call `all()` with no arguments.

See [Reflection — `WideReflectionUnexpectedArgument`](reflection.md#widereflectionunexpectedargument).

---

### `WideReflectionUnknownAccessor`

**When:** An unknown accessor is used under `smelt.models.*` or `smelt.sources.*` (e.g. `smelt.models.bogus()`).

**Message:** `unknown wide-reflection accessor {name}; expected one of: with_tag, all`

**Fix:** use only `with_tag('tag')` or `all()`.

See [Reflection — `WideReflectionUnknownAccessor`](reflection.md#widereflectionunknownaccessor).
