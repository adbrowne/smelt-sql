# Reducers

The meta-language provides **contextual reducers** — a closed set of seven identifiers that can be passed as the second argument to `reduce`. Each reducer folds a `List<T>` into a single SQL fragment of a declared output sort.

Reducers are compile-time constructs. The database engine sees the already-folded SQL fragment, never the `reduce(...)` call itself.

## Usage

```sql
reduce(xs, reducer_name)
```

`reducer_name` is a bare identifier (not a variable, not a string). It is resolved at type-check time from the closed registry. See [Higher-Order Functions](hofs.md) for full details on `reduce`.

## The closed registry

The v1 reducer registry contains exactly seven reducers. Adding a reducer requires a compiler change and a spec edit — the registry is not user-extensible.

---

### `and_all`

Folds a list of Boolean expressions with `AND`.

| Property | Value |
|----------|-------|
| Input | `List<Expr<Boolean>>` |
| Output | `Expr<Boolean>` |
| Empty-list identity | `TRUE` literal |
| Fold formula | `e1 AND e2 AND … AND eN` |

**Example:**

```sql
-- examples/meta_hofs/models/and_all_predicates.sql
-- reduce([true, false, true], and_all) → true AND false AND true
SELECT reduce([true, false, true], and_all)
```

**Typical use:** combine a list of WHERE-clause predicates.

```sql
SELECT id, name
FROM smelt.sources.raw.users
WHERE reduce([is_active, age > 18], and_all)
-- Engine sees: WHERE is_active AND age > 18
```

---

### `comma_sep`

Folds a list of expressions into a comma-separated select-item list.

| Property | Value |
|----------|-------|
| Input | `List<Expr<T>>` (any `T`) |
| Output | `SelectItems<Scalar>` |
| Empty-list identity | Empty `SelectItems` (adjacent commas elide at splice) |
| Fold formula | `e1, e2, …, eN` |

**Example:**

```sql
-- reduce([1, 2, 3], comma_sep) → 1, 2, 3 as select items
SELECT reduce([1, 2, 3], comma_sep) FROM smelt.sources.raw.users
```

**Note:** per-element type information is preserved at the splice point; the `SelectItems<Scalar>` output sort does not carry a generic element type.

---

### `concat`

Folds a list of text expressions with string concatenation.

| Property | Value |
|----------|-------|
| Input | `List<Expr<Text>>` |
| Output | `Expr<Text>` |
| Empty-list identity | Empty string literal `''` |
| Fold formula | `e1 \|\| e2 \|\| … \|\| eN` |

**Example:**

```sql
-- reduce(['hello', ' ', 'world'], concat) → 'hello' || ' ' || 'world'
SELECT reduce(['hello', ' ', 'world'], concat)
```

---

### `intersect_all`

Folds a list of table expressions with `INTERSECT ALL`.

| Property | Value |
|----------|-------|
| Input | `List<TableExpr>` |
| Output | `TableExpr` |
| Empty-list identity | **None** — emits `ReducerEmptyNoIdentity` on an empty list |
| Fold formula | `e1 INTERSECT ALL e2 INTERSECT ALL … INTERSECT ALL eN` |

**Example:**

```sql
SELECT *
FROM reduce(
    [smelt.ref('active_users'), smelt.ref('premium_users')],
    intersect_all
)
-- Engine sees: FROM (active_users INTERSECT ALL premium_users)
```

!!! note
    Because `intersect_all` has no identity for an empty list, always ensure the source list is non-empty. Use `filter` to drop conditionally-empty sublists before reducing.

---

### `or_any`

Folds a list of Boolean expressions with `OR`.

| Property | Value |
|----------|-------|
| Input | `List<Expr<Boolean>>` |
| Output | `Expr<Boolean>` |
| Empty-list identity | `FALSE` literal |
| Fold formula | `e1 OR e2 OR … OR eN` |

**Example:**

```sql
-- reduce([is_admin, is_moderator], or_any) → is_admin OR is_moderator
SELECT id, name
FROM smelt.sources.raw.users
WHERE reduce([is_admin, is_moderator], or_any)
```

---

### `plus_chain`

Folds a list of numeric expressions with addition.

| Property | Value |
|----------|-------|
| Input | `List<Expr<Numeric>>` (any numeric sort; LUB-promoted on output) |
| Output | `Expr<Numeric>` (LUB of input element types) |
| Empty-list identity | `0` cast to the LUB element type |
| Fold formula | `e1 + e2 + … + eN` |

**Example:**

```sql
-- examples/meta_hofs/models/comma_sep_select_list.sql
-- reduce([1, 2, 3], plus_chain) → 1 + 2 + 3
SELECT reduce([1, 2, 3], plus_chain)
```

---

### `union_all`

Folds a list of table expressions with `UNION ALL`.

| Property | Value |
|----------|-------|
| Input | `List<TableExpr>` |
| Output | `TableExpr` |
| Empty-list identity | **None** — emits `ReducerEmptyNoIdentity` on an empty list |
| Fold formula | `e1 UNION ALL e2 UNION ALL … UNION ALL eN` |

**Example:**

```sql
SELECT *
FROM reduce(
    [smelt.ref('orders_2024'), smelt.ref('orders_2025')],
    union_all
)
-- Engine sees: FROM (orders_2024 UNION ALL orders_2025)
```

!!! note
    Like `intersect_all`, `union_all` has no identity for an empty list. Ensure the source list is non-empty, or add a guard with `filter` before reducing.

---

## Reserved names

All seven reducer names are **reserved** at the meta-namespace level. A `smelt.define` function declared with a reducer name emits `ReducerNameShadowed`. Reserved names also cannot be used as `smelt.define` parameter names.

Reserved names: `comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`.

## Diagnostic codes

---

!!! warning "ReducerInputTypeMismatch"
    **When it fires:** `reduce` is called with a list whose element type is incompatible with the reducer's declared input.

    **Message:** `reducer {r} expects List<{T_in}>; found List<{T_actual}>`

    **Fires at:** the `reduce` argument expression.

    **Example:**
    ```sql
    -- and_all expects List<Expr<Boolean>>; [1, 2, 3] is List<Expr<INTEGER>>
    -- ← ReducerInputTypeMismatch
    SELECT reduce([1, 2, 3], and_all)
    ```

    **What to fix:** Check the reducer's declared input type in the table above. Use a lambda inside `map` to convert the list elements to the correct type before reducing, or choose a different reducer that accepts your element type. For example, to reduce integers, use `plus_chain`; to reduce booleans, use `and_all` or `or_any`.

---

!!! warning "ReducerEmptyNoIdentity"
    **When it fires:** `reduce` is called with an empty list using `union_all` or `intersect_all`, which have no identity element for an empty list.

    **Message:** `reducer {r} has no identity for an empty list`

    **Fires at:** the `reduce` call site.

    **Example:**
    ```sql
    -- ← ReducerEmptyNoIdentity: union_all has no identity for an empty list
    SELECT * FROM reduce([], union_all)
    ```

    **What to fix:** Ensure the source list is non-empty before calling `reduce` with `union_all` or `intersect_all`. If the list might be empty at compile time, add a guard: use `filter` to check, or restructure the list construction so it always has at least one element. The other five reducers (`comma_sep`, `and_all`, `or_any`, `plus_chain`, `concat`) do have empty-list identities and are safe with an empty list.

---

!!! warning "ReducerNameShadowed"
    **When it fires:** A `smelt.define` function is declared with a name that matches one of the seven reserved reducer names.

    **Message:** `{name} is a reserved reducer name`

    **Fires at:** the declaration's name token.

    **Example:**
    ```sql
    -- ← ReducerNameShadowed: 'concat' is a reserved reducer name
    smelt.define concat(xs: List<Expr<Text>>) -> Expr<Text> ...
    ```

    **What to fix:** Rename your `smelt.define` function. Choose a name that does not conflict with any of the seven reserved reducer names or the three reserved HOF names (`map`, `filter`, `reduce`).
