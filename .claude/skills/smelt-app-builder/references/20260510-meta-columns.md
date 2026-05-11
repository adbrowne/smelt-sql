# Phase C: smelt.columns_of and ColumnRef reflection

When you need to apply a transformation to every column of a model at compile
time — for example, wrapping all numeric columns in `COALESCE(col, 0)` — Phase C
gives you the reflection primitives to do it without repeating yourself.

## When to use

- Enumerate the columns of a model at compile time: `smelt.columns_of(t)`
- Filter to only numeric columns: pipe into `filter(fn c => c.is_numeric)`
- Generate one SQL expression per column: pipe into `map(fn c => COALESCE(c.name, 0))`
- Define a reusable helper function that works over any model's schema

## Surface (Phase C)

### smelt.columns_of

```sql
smelt.columns_of(t)
```

- Takes exactly one positional argument: a `TableExpr` parameter or a smelt model
  path (e.g. `smelt.orders`)
- Returns `List<ColumnRef>` — one element per column in the model's schema,
  in declared column order
- Named arguments are not supported: `smelt.columns_of(t => orders)` is a compile-time error
- The argument must resolve to a model whose schema is statically known; if the
  schema cannot be resolved, `ColumnsOfUnresolvableSchema` fires at compile time

### ColumnRef fields (closed set)

Each element of the `List<ColumnRef>` returned by `smelt.columns_of` exposes
exactly three fields:

| Field | Type | Description |
|-------|------|-------------|
| `c.name` | `Text` | The column name as a SQL identifier (lifted to SQL at expansion time) |
| `c.type` | `Text` | The column's declared data type as a string |
| `c.is_numeric` | `Boolean` | `true` when the column is a numeric type |

Accessing any field not in this set (`c.foo`, `c.label`, etc.) is a compile-time
error: `ColumnRefFieldUnknown`.

### Getting a ColumnRef binding

`ColumnRef` is **not a user-writable parameter type** in `smelt.define`. The only
way to bind a `ColumnRef`-typed value is via a HOF lambda over a `List<ColumnRef>`
source list — i.e., inside `map(fn c => …)` or `filter(fn c => …)` where the first
argument is `smelt.columns_of(t)`. The compiler binds `c` as `ColumnRef`-typed for
you; you do not write `c: ColumnRef` anywhere.

Accessing a field not in the closed set (`c.foo`, `c.label`, etc.) is a compile-time
error: `ColumnRefFieldUnknown`.

### Worked example: coalesce_numeric

```sql
-- functions/coalesce_numeric.sql
smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (
    smelt.columns_of(t)
      |> filter(fn c => c.is_numeric)
      |> map(fn c => COALESCE(c.name, 0))
)
```

```sql
-- models/orders_safe.sql
SELECT
    customer_name,
    ...smelt.functions.coalesce_numeric(smelt.orders)
FROM smelt.orders
```

At expansion time, smelt resolves the schema of `smelt.orders`, runs
`columns_of`, applies the filter and map, and splices the result into the
SELECT list in place of the spread.

## Phase C diagnostics

| Code | Trigger |
|------|---------|
| `ColumnsOfRequiresTableExpr` | `smelt.columns_of(42)` — argument is not a model reference or `TableExpr` param |
| `ColumnsOfNamedArgument` | `smelt.columns_of(t => orders)` — named arguments are not supported |
| `ColumnsOfUnresolvableSchema` | `smelt.columns_of(nonexistent)` — the model does not exist or has no resolvable schema |
| `ColumnRefFieldUnknown` | `c.foo` where `foo` is not in the closed field set `{name, type, is_numeric}` |

## Workflow gotchas

- **smelt.columns_of is a meta-builtin, not a SQL function**: it does not appear
  in the builtin function registry and is not callable as a scalar expression in
  arbitrary contexts. It must appear as the left-hand side of a pipe chain or
  directly in a HOF argument position inside a function body.
- **Column ordering is stable**: `smelt.columns_of(t)` always returns columns in
  the order they are declared in the model's schema YAML.
- **c.name is a meta-Text value**: at expansion time `c.name` is lifted to a SQL
  identifier — you get the column name as a SQL expression, not a string literal.
  Do not quote it: `'c.name'` is a string literal, not the column name.
- **TableExpr bodies are checked at call-site**: because `smelt.define` functions
  with `TableExpr` params need the caller's schema to type-check the body,
  definition-time checking is skipped for those functions. Errors surface at the
  call site instead.
- **ColumnRef field errors surface in HOF lambda bodies**: `ColumnRefFieldUnknown`
  fires inside `map(fn c => …)` or `filter(fn c => …)` where the source list
  comes from `smelt.columns_of(t)`. The compiler detects the closed field set
  violation at the HOF call site.

See `docs-site/docs/meta-language/columns.md` for full surface and worked examples.
