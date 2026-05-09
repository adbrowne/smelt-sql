# Meta-Language Reference

Alphabetical quick reference for all shipped meta-language constructs and diagnostic codes. Phase A entries are populated below; later phases append new entries as they ship.

For a conceptual introduction, see [Overview](index.md). For worked examples and a full explanation of list and spread behaviour, see [Lists & Spread](lists.md).

---

## `...xs` — spread operator

**Type:** consumes `List<T>`; materialises `T` elements into the surrounding comma-separated grammar position.

**Syntax:**
```
...expr
```

where `expr` evaluates to a `List<T>`.

**Valid positions (Phase A):** SELECT lists.

**Valid positions (Phase B, planned):** GROUP BY, ORDER BY, positional function arguments, IN-lists, VALUES rows, inside other list literals.

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

## `List<T>` — meta list type

**Kind:** meta-only type; never appears in data-world SQL.

**Definition:** a finite, ordered, immutable sequence of elements of type `T`. Length is fixed at construction. `T` is a fragment sort (`Expr<U>`, `OrderSpec`, …) or a data-type lifted as a meta literal (`Text`, `Integer`, …).

**Covariance:** `List<S> <: List<T>` whenever `S <: T`. Sound because lists are immutable.

**Construction:** list literals `[a, b, c]` (Phase A); HOFs `map`, `filter` (Phase B).

**Hover:** hovering over a list literal shows `List<T>` with `T` resolved to the inferred element type, e.g. `List<Expr<INTEGER>>`.

**Example:**
```sql
SELECT ...[1, 2, 3] FROM smelt.sources.raw.users
--     ^^^^^^^^^^
--     List<Expr<INTEGER>> — hover shows this type in the editor
```

See [Lists & Spread — The `List<T>` type](lists.md#the-listt-type) for the covariance rule and subtyping details.

---

## Diagnostic codes

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

**Fix:** move the spread to a SELECT list. For WHERE-clause predicate lists, use the `and_all` reducer (Phase B). For IN-list membership, use `WHERE id IN (...vs)` (Phase B).

!!! note
    In Phase A, forbidden positions other than WHERE may emit parse errors rather than this diagnostic. The full set of friendly diagnostics for forbidden positions lands in Phase B.

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
