# Higher-Order Functions

The meta-language provides three built-in **higher-order functions (HOFs)**: `map`, `filter`, and `reduce`. HOFs operate on [`List<T>`](lists.md) values at compile time and produce new `List<U>` values or reduced fragments that splice into SQL. Each HOF takes a [lambda](lambdas.md) or a [reducer](reducers.md) as its second argument.

All three HOFs are meta-world only. Their inputs and outputs are compile-time values; the database engine sees the already-reduced SQL fragments, never the HOF calls themselves.

## The three HOFs at a glance

| HOF | Signature | Returns |
|-----|-----------|---------|
| `map` | `(xs: List<T>, f: Lambda<T, U>) -> List<U>` | A new list of the same length as `xs`, where each element is the result of applying `f` |
| `filter` | `(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>` | A sub-list of `xs` (in original order), keeping only elements for which `p` returns `TRUE` |
| `reduce` | `(xs: List<T>, r) -> OutputSort` | A single fragment — the result of folding `xs` left-to-right using reducer `r` |

HOFs accept exactly **two positional arguments** and **zero named arguments**. The `=>` that appears in a lambda body (`fn x => expr`) is part of the lambda syntax; it is not a named argument.

## `map`

`map(xs, fn x => body)` applies a lambda to every element of a list and returns a new list of the same length.

**Type signature:**
```
map(xs: List<T>, f: Lambda<T, U>) -> List<U>
```

**Worked example** — drawn from `examples/meta_hofs/`:

```sql
-- examples/meta_hofs/models/pipe_rewrite.sql
-- map(xs, fn c => c * 2) doubles every element.
-- [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- desugars to: map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
```

**Behaviour:**
- Walks `xs` left-to-right exactly once.
- The lambda body is type-checked once (parametrically, under the HOF's `T`) and evaluated once per element.
- Order is preserved: `map([a, b, c], f)` produces `[f(a), f(b), f(c)]`.
- The result list has the same length as `xs`. `map` never drops or duplicates elements.

**Editor support:** hovering over a `map(...)` call shows `List<U>` with `U` resolved to the lambda body's inferred type.

## `filter`

`filter(xs, fn x => predicate)` keeps only the elements of a list for which the predicate returns `TRUE`.

**Type signature:**
```
filter(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>
```

**Worked example:**

```sql
-- Keep only the positive numbers.
-- After compile, the filtered list [1, 2, 3] (all positives) is produced.
SELECT [1, 2, 3] |> filter(fn c => c > 0)
```

**Behaviour:**
- The predicate body must synthesise to `Boolean`. A non-Boolean predicate emits `LambdaResultTypeMismatch`.
- An element is kept if and only if the predicate is `TRUE`. A predicate that evaluates to `Unknown` drops the element silently (no error).
- Order is preserved: retained elements appear in their original order.
- The result is a `List<T>` — the same element type as `xs`.

**Editor support:** hovering over a `filter(...)` call shows `List<T>` with `T` resolved to the input element type.

## `reduce`

`reduce(xs, r)` folds a list into a single fragment using one of the seven registered reducers.

**Type signature:**
```
reduce(xs: List<T>, r) -> OutputSort
```

where `r` is a bare reducer identifier from the [closed registry](reducers.md), and `OutputSort` is the reducer's declared output sort.

**Worked example — `and_all`** (reduces a list of Boolean expressions with `AND`):

```sql
-- examples/meta_hofs/models/and_all_predicates.sql
-- reduce([true, false, true], and_all) folds left: true AND false AND true
SELECT reduce([true, false, true], and_all)
```

**Worked example — `plus_chain`** (reduces a list of numeric expressions with `+`):

```sql
-- examples/meta_hofs/models/comma_sep_select_list.sql
-- reduce([1, 2, 3], plus_chain) folds left: 1 + 2 + 3
SELECT reduce([1, 2, 3], plus_chain)
```

**Behaviour:**
- The reducer identifier is resolved at type-check time from the closed registry. It is not a value-bearing expression — you cannot pass a variable as the reducer argument.
- If `xs` is non-empty, the result is the reducer's left-fold over `xs` in original order.
- If `xs` is empty, the result depends on the reducer's declared identity element. See [Reducers](reducers.md) for the full table.

**Editor support:** hovering over the reducer name in `reduce(xs, here)` shows the reducer's input element type, output sort, and empty-list identity (or "no identity").

## Reserved names

The names `map`, `filter`, and `reduce` are **reserved** at the meta-namespace level. A `smelt.define` function declared with one of these names emits `HofNameShadowed` at the declaration. Reserved names also cannot be used as `smelt.define` parameter names.

This reservation is workspace-wide — it applies even if your workspace has no models that use HOFs.

## Pipe shorthand

The three HOFs chain naturally with the [pipe operator `|>`](pipes.md):

```sql
-- Pipe-chained form
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)

-- Equivalent un-piped form
SELECT map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
```

Both forms are semantically identical. Pipe is purely syntactic sugar; the type checker desugars it before checking.

## Diagnostic codes

---

!!! warning "HofExpectsLambda"
    **When it fires:** The second argument to `map` or `filter` is not a lambda.

    **Message:** `{hof} expects a lambda; found {actual type}`

    **Fires at:** the second argument expression.

    **Example:**
    ```sql
    -- ← HofExpectsLambda: second arg must be a lambda, not an identifier
    SELECT map([1, 2, 3], some_function)
    ```

    **What to fix:** Replace the second argument with a `fn x => body` lambda. For example: `map([1, 2, 3], fn c => c + 1)`. Named `smelt.define` functions cannot be passed as HOF arguments; write the transformation inline as a lambda.

---

!!! warning "HofExpectsReducer"
    **When it fires:** The second argument to `reduce` is not a bare reducer identifier from the closed registry.

    **Message:** `reduce expects a reducer; found {actual}`

    **Fires at:** the second argument span.

    **Example:**
    ```sql
    -- ← HofExpectsReducer: 'my_reducer' is not a registered reducer name
    SELECT reduce([1, 2, 3], my_reducer)
    ```

    **What to fix:** Replace the second argument with one of the seven registered reducer names: `comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, or `concat`. See [Reducers](reducers.md) for the input type and output sort of each. You cannot define your own reducers in v1; the registry is closed.

---

!!! warning "HofNameShadowed"
    **When it fires:** A `smelt.define` function is declared with the name `map`, `filter`, or `reduce`.

    **Message:** `{name} is a reserved higher-order function name`

    **Fires at:** the declaration's name token.

    **Example:**
    ```sql
    -- ← HofNameShadowed: 'map' is a reserved HOF name
    smelt.define map(xs: List<Expr<Integer>>) -> SelectItems<Scalar> ...
    ```

    **What to fix:** Rename your `smelt.define` function. Choose a name that does not conflict with the three reserved HOF names (`map`, `filter`, `reduce`) or any of the [seven reserved reducer names](reducers.md#reserved-names).
