# Lists & Spread

The meta-language provides two constructs for working with compile-time lists: **list literals** for building compile-time lists of SQL fragments, and the **spread operator** for splicing those lists into SELECT lists and other comma-separated grammar positions.

See also: [Higher-Order Functions](hofs.md) for `map`, `filter`, and `reduce` that transform lists; [Pipe Operator](pipes.md) for chaining list operations left-to-right; [Reducers](reducers.md) for folding a list into a single SQL fragment; [Reference](reference.md) for the complete alphabetical construct index.

## The `List<T>` type

`List<T>` is a meta-only type. A `List<T>` value is:

- **Finite** — length is fixed at construction.
- **Ordered** — elements are in declaration order.
- **Immutable** — no element can be added, removed, or replaced after construction.
- **Compile-time-only** — no `List<T>` value ever reaches the database engine.

`T` is a fragment sort: `Expr<U>` for expression elements, `OrderSpec` for order items, or a data type lifted as a meta literal (`Text`, `Integer`, …). `List<T>` is **covariant** in `T`: if `S` is a subtype of `T`, then `List<S>` is a subtype of `List<T>`. Immutability makes this sound.

## List literal syntax `[a, b, c]`

A comma-separated, square-bracketed expression list constructs a `List<T>`:

```sql
-- List<Expr<INTEGER>>: three integer expressions
...[1, 2, 3]

-- List<Expr<…>>: column reference expressions (element type depends on source schema)
...[name, email]

-- Trailing comma is allowed
...[name, email,]

-- Singleton list
...[id]
```

The inferred element type is the **least upper bound (LUB)** of all element types. Numeric promotion applies: `[1, 2.5]` infers `List<Expr<DECIMAL>>`.

**Example — spread a list of columns into a SELECT list:**

```sql
-- examples/meta_lists/models/select_with_spread.sql
-- [name, email] is a List<Expr<…>> meta-literal (element type from source schema).
-- After compilation: SELECT id, name, email FROM smelt.sources.raw.users
SELECT
    id,
    ...[name, email]
FROM smelt.sources.raw.users
```

**Example — multiple spreads in one SELECT list:**

```sql
-- examples/meta_lists/models/multi_spread.sql
-- Two separate spreads expand inline.
-- After compilation: SELECT id, name, email, created_at FROM ...
SELECT
    id,
    ...[name, email],
    created_at
FROM smelt.sources.raw.users
```

**Example — list of integer literals:**

```sql
-- examples/meta_lists/models/list_literal.sql
-- [1, 2, 3] is List<Expr<INTEGER>>.
-- After compilation: SELECT id, 1, 2, 3 FROM ...
SELECT
    id,
    ...[1, 2, 3]
FROM smelt.sources.raw.users
```

## Bidirectional disambiguation

The parser produces a single `LIST_LITERAL` CST node for `[…]`. The type checker decides whether it is a meta list or a data-world array, based on the surrounding context:

| Surrounding context | Result |
|---------------------|--------|
| Target sort is `List<T>` (meta position — e.g. a spread, a meta parameter) | Meta list `List<T>` |
| Target sort is `Expr<Array<U>>` (data-world array position) | Data-world `Array<U>` |
| Both meta and data-world readings are valid | **Meta wins** |
| Neither reading is valid | Type error; literal is `List<Unknown>` |

**Meta context** — SELECT-list splice point, spread a list into a SELECT:

```sql
-- Meta context: ...xs splices List<Expr<INTEGER>> into SELECT
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
-- Engine sees: SELECT 1, 2, 3 FROM ...
```

**Data-world context** — a bare list literal in a SELECT-item position (no spread) where the column type resolves to `Array<U>`:

```sql
-- Data-world context: [1, 2, 3] as a standalone SELECT item is an Array<INTEGER> column.
-- No spread → the list literal is treated as a runtime array value, not a meta splice.
SELECT id, [1, 2, 3] AS scores FROM smelt.sources.raw.users
-- Engine sees: SELECT id, [1, 2, 3] AS scores FROM ...
-- Column `scores` has type Array<INTEGER>.
```

!!! note
    Bidirectional disambiguation is currently wired at SELECT-list positions. IN-list positions (`WHERE id IN ([1, 2, 3])`) are not yet type-checked — the literal passes through unprocessed. Full disambiguation for IN-lists and other positions is planned but not yet implemented.

## Spread operator `...xs`

`...xs` splices the elements of a `List<T>` into the surrounding comma-separated grammar position. Each element is emitted at the spread's source span and validated against the surrounding position's type rules.

**Where spread works:**

- SELECT lists — fully type-checked.

**Planned but not yet supported** — spread in the following positions is planned but not yet wired:

- GROUP BY clauses.
- ORDER BY clauses (list element type must be `OrderSpec`).
- Positional argument positions of function calls.
- IN-lists (`x IN (...vs)`).
- VALUES rows.
- Inside other list literals (`[a, ...xs, b]`).

**Hover** — when your editor's cursor is on a spread operator, hover shows the source list type. For example, `...[1, 2, 3]` shows `List<Expr<INTEGER>>`.

## Empty-list semantics

An empty list literal `[]` requires a surrounding context that supplies a target sort. In a SELECT-list splice point without a target, the type checker cannot infer the element type and emits `MetaListEmptyTypeUnknown`.

An empty spread `...[]` is **silently elided** — it disappears along with its adjacent commas. This is not an error.

```sql
-- examples/meta_lists/models/empty_spread.sql
-- ...[] elides itself and its adjacent commas.
-- After compilation: SELECT id, created_at FROM ...
SELECT
    id,
    ...[],
    created_at
FROM smelt.sources.raw.users
```

!!! tip
    If you want an empty list that causes a diagnostic (for testing), use `[]` as a bare SELECT item without a spread. If you want silent elision, use `...[]`.

## Diagnostic codes

The list and spread surface introduces four diagnostic codes. All are anchored at the offending CST span in your editor.

---

!!! warning "MetaListEmptyTypeUnknown"
    **When it fires:** A bare `[]` list literal appears in a position where the type checker cannot infer the element type.

    **Message:** `cannot infer element type for empty list literal`

    **Example:**
    ```sql
    -- examples/meta_lists_broken_empty_unknown/models/empty_unknown.sql
    SELECT
        id,
        []       -- ← MetaListEmptyTypeUnknown: no target sort here
    FROM smelt.sources.raw.users
    ```

    **What to fix:** Either provide elements so the type can be inferred, or use `...[]` (spread of an empty list) if your intent is to emit nothing. If you are building the list in a `smelt.define` parameter, annotate the parameter with the expected `List<T>` type.

---

!!! warning "MetaListHeterogeneous"
    **When it fires:** The elements of a list literal do not share a common type under LUB.

    **Message:** `list elements have incompatible types: {T0}, {Tk}`

    **Example:**
    ```sql
    -- examples/meta_lists_broken_heterogeneous/models/heterogeneous.sql
    SELECT
        id,
        ...[1, 'hello']  -- ← MetaListHeterogeneous: INTEGER and TEXT don't unify
    FROM smelt.sources.raw.users
    ```

    **What to fix:** Ensure all list elements have compatible types. For numeric mixed precision (`[1, 2.5]`), smelt promotes to the wider type automatically. For truly mixed types (integer and text), split into separate lists or cast all elements to a common type:
    ```sql
    -- Cast to a common type
    SELECT id, ...[CAST(1 AS TEXT), 'hello'] FROM smelt.sources.raw.users
    ```

---

!!! warning "MetaSpreadInForbiddenPosition"
    **When it fires:** A spread operator `...xs` appears in a position that does not support spread.

    **Message:** `spread is not allowed in {position name}`

    **Forbidden positions:**
    - WHERE clauses — no comma-separated grammar; use the [`and_all`](reducers.md#and_all) reducer.
    - FROM clauses without an explicit reducer — no default join semantics for `List<TableExpr>`.
    - Boolean-composition contexts (`x AND ...preds`, `y OR ...preds`).
    - Named-argument positions — spread cannot stand on the left of `=>`.

    **Example:**
    ```sql
    -- examples/meta_lists_broken_spread_forbidden/models/spread_forbidden.sql
    SELECT id
    FROM smelt.sources.raw.users
    WHERE id = 1 AND ...preds  -- ← MetaSpreadInForbiddenPosition
    ```

    **What to fix:** Move the spread to a SELECT list. To combine a list of predicates in a WHERE clause, use the `and_all` reducer. If the intent is to filter by a list of values, use `IN (...vs)` with a spread (planned but not yet implemented):
    ```sql
    -- Planned syntax (not yet implemented):
    WHERE id IN (...id_list)
    ```

    !!! note
        Forbidden positions other than WHERE clauses may currently produce parse errors rather than the `MetaSpreadInForbiddenPosition` diagnostic. The friendly diagnostic for all forbidden positions is planned but not yet wired everywhere.

---

!!! warning "MetaSpreadOnNonList"
    **When it fires:** The `...` operator is applied to a value that is not a `List<T>`.

    **Message:** `spread expects List<T>; found {actual type}`

    **Example:**
    ```sql
    SELECT
        id,
        ...some_integer   -- ← MetaSpreadOnNonList: INTEGER is not a List<T>
    FROM smelt.sources.raw.users
    ```

    **What to fix:** Wrap the value in a list literal if you want to splice a single element: `...[some_integer]`. Or, if the variable is supposed to be a list, check the type of the binding that supplies it.

---

!!! warning "MetaListInScalarPosition"
    **When it fires:** A `List<T>` reaches a scalar / SELECT-item position without being consumed — by a spread, a higher-order function, a reducer, a record, a map, or a generator. A list cannot become a scalar value on its own, and there is no implicit auto-spread.

    **Message:** `a List<T> cannot be used as a scalar value here; consume it with a spread (...xs), a reducer (reduce(xs, …)), or a HOF before splicing`

    **Example:**
    ```sql
    SELECT [1, 2, 3]                       -- ← MetaListInScalarPosition (bare list)
    SELECT xs |> map(fn c => c * 2)        -- ← MetaListInScalarPosition (map result left bare)
    ```

    **What to fix:** Consume the list. Spread it into columns (`SELECT ...[1, 2, 3]`), or fold it to a scalar with a reducer (`SELECT reduce([1, 2, 3], plus_chain)`). This check runs in every model, including a model with no `FROM` clause.
