# Reflection

smelt's **reflection** surface lets you inspect the column schema of a model, source, or seed at compile time and turn that schema into SQL — without manually listing column names. The reflection API operates entirely in the meta-world: no column name ever reaches the database engine as a literal string; it is lifted to a SQL identifier at the splice point.

This page covers the narrow `smelt.columns_of` accessor, which reflects a single `TableExpr`-typed value into a `List<ColumnRef>`. Workspace-wide reflection (`smelt.models.*`, `smelt.sources.*`) is planned but not yet implemented — see [Planned but not yet implemented](#planned-but-not-yet-implemented).

## `smelt.columns_of`

### Signature

```
smelt.columns_of(t: TableExpr) -> List<ColumnRef>
```

`smelt.columns_of` is a meta-only accessor. It takes exactly one positional argument — a `TableExpr`-typed value — and returns the column list of that table as a `List<ColumnRef>`.

The argument may be any of the following:

- **A `smelt.<path>` reference** resolving to a model, source, or seed. The existing schema-resolution machinery supplies the column list from the target's declared schema.
- **A `smelt.define` parameter** declared `TableExpr` or `TableExpr<{…}>`. At function body-check time the result type is `List<ColumnRef>` parametrically — the concrete column list is not materialised until expansion time at each call site. A `TableExpr<{required columns}>` parameter contributes only the required columns to body-check-time reasoning; at expansion time the call-site argument's full schema (which may include additional columns under the row-tail) supplies the complete list.
- **A CTE alias or subquery alias** within the same model body, resolved through the standard `TableExpr`-typed expression path.

`smelt.columns_of` must be called with **exactly one positional argument**. Named arguments emit `ColumnsOfNamedArgument`. An argument whose type is not assignable to `TableExpr` emits `ColumnsOfRequiresTableExpr`. If the schema cannot be resolved at expansion time (for example because an upstream model has an `Unknown` schema), `ColumnsOfUnresolvableSchema` is emitted and the surrounding HOF call drops its splice without further diagnostics.

The returned list preserves the **declared column order** of the source schema. For models, sources, and seeds this is the order columns appear in their schema; for `TableExpr` parameters at expansion time this is the order columns appear in the call-site argument's schema.

`smelt.columns_of` is Salsa-cached: given the same workspace snapshot, it returns byte-equal results. LSP invalidation is automatic when an upstream schema changes.

### Example — list all columns

```sql
-- List all columns of the orders model.
-- At expansion time, smelt.columns_of(smelt.orders) produces a List<ColumnRef>
-- whose elements correspond to the four columns declared in orders.sql.
SELECT
    ...smelt.columns_of(smelt.orders) |> map(fn c => c.name)
FROM smelt.orders
```

For a fuller worked example, see [Worked example: `coalesce_numeric`](#worked-example-coalesce_numeric) below.

---

## `ColumnRef`

`ColumnRef` is a **closed meta-only record type** that represents a single column from a resolved schema. You cannot construct a `ColumnRef` directly — values originate only from `smelt.columns_of`.

### Fields

| Field | Type | Meaning |
|---|---|---|
| `name` | `Text` | The column's identifier as it appears in the source schema (un-quoted; case-preserved) |
| `type` | `DataType` | The column's `DataType` from the `DataType` vocabulary |
| `is_numeric` | `Boolean` | `TRUE` if `type` is in the `Numeric` constraint set |

Access each field using dot-notation inside a HOF lambda:

```sql
smelt.columns_of(smelt.orders)
  |> filter(fn c => c.is_numeric)        -- Boolean field
  |> map(fn c => c.name)                 -- Text field, lifts to identifier in splice
```

Any field name other than `name`, `type`, or `is_numeric` emits `ColumnRefFieldUnknown`.

`ColumnRef` is **closed**: the field set is exactly the three fields above. Adding a field requires a spec edit and a compiler change. `ColumnRef` is also **not user-writable** — you cannot use it as a `smelt.define` parameter or return type annotation, and you cannot construct a `ColumnRef` value in a list literal.

### Body-check vs expansion-time

Inside a `smelt.define` function body where the argument to `smelt.columns_of` is a `TableExpr` parameter, the type checker operates in two regimes:

- **At body-check time**: `smelt.columns_of(t)` synthesises as `List<ColumnRef>` parametrically. Each HOF lambda over the result is type-checked once against `ColumnRef`. No concrete column list is materialised.
- **At expansion time**: when the function is inlined at a call site with a concrete `TableExpr` argument, the call-site schema is resolved, the `List<ColumnRef>` is materialised, HOF lambdas walk each element, and meta-`Text`-as-identifier lifts are validated against the surrounding splice context.

This means you get full type-safety at definition time (the type checker knows each lambda parameter is a `ColumnRef` and that `c.name` is `Text`, `c.is_numeric` is `Boolean`, etc.) while the concrete expansion is deferred until each call site provides a schema.

---

## Meta-`Text`-as-identifier lift

A `ColumnRef`'s `name` field has type `Text`. When a meta-`Text` value — such as `c.name` inside a HOF lambda — appears in a position where the SQL grammar expects an unquoted **identifier**, smelt lifts that value to the identifier rather than treating it as a string literal.

The lift applies in **exactly four positions**:

| Position | Example |
|---|---|
| Column-reference position inside an expression | `COALESCE(c.name, 0)` — `c.name` lifts to a column reference |
| `AS` alias of a SELECT item | `SUM(amount) AS c.name` — `c.name` lifts to the output column alias |
| `ORDER BY` column reference | `ORDER BY c.name` — `c.name` lifts to a sort key |
| `GROUP BY` column reference | `GROUP BY c.name` — `c.name` lifts to a grouping key |

In **any other position** — function arguments typed `Expr<Text>`, comparison operands typed `Text`, string-literal positions, named-argument values — a meta-`Text` value retains its string-value meaning. The lift is grammar-position-driven; there is no annotation or opt-in marker.

After lifting, the identifier is validated against the surrounding splice context's column-resolution scope using the standard scoping rule. If the lifted identifier names no in-scope column, the existing `UnknownColumn` diagnostic fires at the meta expression's source span (not at the lifted text). The lift itself produces no additional diagnostic.

The lift applies **only to compile-time meta-`Text` values**. A runtime `Expr<Text>` (for example `UPPER('foo')`) in an identifier position remains a data-world type error; the meta lift does not extend to evaluated SQL expressions.

### Examples

```sql
-- Column-reference lift: c.name becomes the column identifier
smelt.columns_of(smelt.orders)
  |> map(fn c => COALESCE(c.name, 0))
-- c.name (meta-Text) lifts to the column identifier: COALESCE(id, 0), COALESCE(amount, 0), …

-- AS-alias lift: c.name becomes the SELECT alias
smelt.columns_of(smelt.orders)
  |> map(fn c => COALESCE(c.name, 0) AS c.name)
-- c.name after AS lifts to alias: COALESCE(id, 0) AS id, COALESCE(amount, 0) AS amount, …

-- GROUP BY lift
smelt.columns_of(smelt.orders)
  |> filter(fn c => NOT c.is_numeric)
  |> map(fn c => c.name)
-- The spread of this list into GROUP BY: GROUP BY customer_name
```

---

## Worked example: `coalesce_numeric`

This example mirrors the fixture in `examples/meta_columns/`.

### The upstream model: `orders.sql`

```sql
-- examples/meta_columns/models/orders.sql
-- Schema: id INTEGER, customer_name VARCHAR, amount DOUBLE, discount DOUBLE
SELECT
    id,
    customer_name,
    amount,
    discount
FROM smelt.sources.raw.orders
```

The `orders` model has four columns. Three are numeric (`id`, `amount`, and `discount`); one is non-numeric (`customer_name`).

### The function: `coalesce_numeric.sql`

```sql
-- examples/meta_columns/functions/coalesce_numeric.sql
smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (
    smelt.columns_of(t)
      |> filter(fn c => c.is_numeric)
      |> map(fn c => COALESCE(c.name, 0))
)
```

**What the type checker sees at body-check time:**

- `smelt.columns_of(t)` synthesises `List<ColumnRef>` parametrically. `t` is a `TableExpr` parameter — no concrete schema is available yet.
- `filter(fn c => c.is_numeric)`: the lambda parameter `c` is typed `ColumnRef`; `c.is_numeric` synthesises `Boolean`. Type-checks cleanly.
- `map(fn c => COALESCE(c.name, 0))`: `c.name` synthesises `Text`. In the column-reference position inside `COALESCE(…)`, a meta-`Text` value is lifted to an identifier. The checker records that the lift will be validated at expansion time. `COALESCE(text_identifier, 0)` type-checks against the return sort `SelectItems<Scalar, t>`.

**What happens at expansion time (when called with `smelt.orders`):**

1. `t` is bound to `smelt.orders`, whose schema is `{id: INTEGER, customer_name: VARCHAR, amount: DOUBLE, discount: DOUBLE}`.
2. `smelt.columns_of(smelt.orders)` materialises `[{name:"id", type:Integer, is_numeric:TRUE}, {name:"customer_name", type:Text, is_numeric:FALSE}, {name:"amount", type:Double, is_numeric:TRUE}, {name:"discount", type:Double, is_numeric:TRUE}]`.
3. `filter(fn c => c.is_numeric)` keeps `id`, `amount`, `discount`.
4. `map(fn c => COALESCE(c.name, 0))` lifts each `c.name` to a column identifier, producing `COALESCE(id, 0)`, `COALESCE(amount, 0)`, `COALESCE(discount, 0)`.

### The caller: `orders_safe.sql`

```sql
-- examples/meta_columns/models/orders_safe.sql
SELECT
    customer_name,
    ...smelt.functions.coalesce_numeric(smelt.orders)
FROM smelt.orders
```

**What the engine sees after compilation:**

```sql
SELECT
    customer_name,
    COALESCE(id, 0),
    COALESCE(amount, 0),
    COALESCE(discount, 0)
FROM smelt.orders
```

The spread `...smelt.functions.coalesce_numeric(smelt.orders)` materialises the three `SelectItems` produced by the function inline into the SELECT list. No column name is ever a string literal in the output — the meta-`Text` values carried by `c.name` are lifted to identifiers during expansion.

---

## LSP support

**Editor support:** reflection constructs surface in hover, completion, and diagnostics.

- **Hover** on `smelt.columns_of(t)` shows `List<ColumnRef>`. When `t`'s schema is statically resolvable (for example a direct `smelt.<path>` reference), the tooltip also shows the resolved column count and the first five column names.
- **Hover** on a `ColumnRef`-typed lambda parameter (for example `c` in `fn c => c.name`) shows `ColumnRef` together with the closed field list and each field's type.
- **Hover** on a field projection (`c.name`, `c.type`, `c.is_numeric`) shows the field's declared type. When the projection is reached at expansion time over a resolvable list, the tooltip shows the concrete value at the current call site.
- **Goto-definition** on `smelt.columns_of` resolves to the reference page as a URL hint; clients that do not support URL goto-definition targets no-op gracefully.
- **Completion** at a field-projection position (`c.<cursor>`) offers the three closed field names: `name`, `type`, `is_numeric`.
- **Completion** at the `smelt.columns_of(<cursor>)` argument position offers in-scope `TableExpr`-valued names — `smelt.<path>` references and the enclosing function's `TableExpr` parameters.
- **Diagnostics with frame stacks**: a type error inside a HOF lambda body whose source list comes from `smelt.columns_of(t)` carries an anonymous expansion frame. When the source column is statically traceable, the frame includes a `column_origin` field pointing to the column's declaration span in the upstream schema, surfaced as a "from column declared at …" trailer.

---

## Diagnostic codes

---

!!! warning "ColumnsOfRequiresTableExpr"
    **When it fires:** `smelt.columns_of(x)` is called and `x` synthesises to a type that is not assignable to `TableExpr`.

    **Message:** `smelt.columns_of expects TableExpr; found {actual}`

    **Fires at:** the argument expression.

    **Example:**
    ```sql
    -- ← ColumnsOfRequiresTableExpr: INTEGER is not TableExpr
    smelt.columns_of(42)
    ```

    **What to fix:** Pass a `TableExpr`-typed value — a `smelt.<path>` reference to a model, source, or seed, or a `TableExpr` parameter of the enclosing `smelt.define` function. Use `smelt.columns_of(smelt.orders)` rather than passing a non-table expression.

---

!!! warning "ColumnsOfNamedArgument"
    **When it fires:** `smelt.columns_of` is called with a named argument instead of a positional argument.

    **Message:** `smelt.columns_of takes one positional argument; named arguments are not supported`

    **Fires at:** the named-argument span.

    **Example:**
    ```sql
    -- ← ColumnsOfNamedArgument: named argument not supported here
    smelt.columns_of(t => smelt.orders)
    ```

    **What to fix:** Remove the `=>` and pass the value positionally: `smelt.columns_of(smelt.orders)`.

---

!!! warning "ColumnsOfUnresolvableSchema"
    **When it fires:** At expansion time, `smelt.columns_of(t)` is evaluated but the schema for `t` cannot be statically determined (for example because an upstream model has an unknown schema).

    **Message:** `cannot resolve column list for {t}; upstream schema is unknown`

    **Fires at:** the `smelt.columns_of(…)` call site.

    **What to fix:** Ensure the `TableExpr` argument resolves to a model, source, or seed with a fully declared schema. If the upstream model itself has type errors, fix those first — the schema becomes resolvable once the upstream compiles cleanly. This diagnostic suppresses further errors from the surrounding HOF call; fix the schema resolution first, then re-check.

---

!!! warning "ColumnRefFieldUnknown"
    **When it fires:** Field access on a `ColumnRef`-typed value uses an identifier that is not one of the three declared fields.

    **Message:** `ColumnRef has no field {name}; expected one of: name, type, is_numeric`

    **Fires at:** the field name span in the dot-notation expression.

    **Example:**
    ```sql
    smelt.columns_of(smelt.orders)
      |> map(fn c => c.label)   -- ← ColumnRefFieldUnknown: 'label' is not a ColumnRef field
    ```

    **What to fix:** Use one of the three valid field names: `c.name` (the column's identifier as `Text`), `c.type` (the column's `DataType`), or `c.is_numeric` (`Boolean`). If you need metadata beyond these three fields, that requires a spec extension.

---

## Planned but not yet implemented

The following reflection capabilities are planned but not yet available:

- **Workspace-wide reflection** (`smelt.models.*`, `smelt.sources.*`, `ModelRef`): iterate over all models or all sources in the workspace. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Record types and config loaders** (`Record<{…}>`, `Map<K,V>`, YAML/JSON/TOML loaders): user-writable record types and structured config loading.
- **Multi-model production**: one file generates multiple output models.

## See also

- [Lists & Spread](lists.md) — `List<T>` type, list literals, and the spread operator used to materialise reflection results into SELECT lists.
- [Higher-Order Functions](hofs.md) — `filter` and `map`, which are the primary tools for transforming a `List<ColumnRef>`.
- [Pipe Operator](pipes.md) — `|>` for readable HOF chains over `smelt.columns_of` results.
- [Reference](reference.md) — alphabetical quick reference including all reflection diagnostic codes.
