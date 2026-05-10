# Lambdas

Phase B of the meta-language introduces **lambda expressions** — anonymous single-argument functions written inline as arguments to `map` or `filter`. (`reduce`'s second argument is a bare reducer identifier, not a lambda — see [Reducers](reducers.md).) A lambda lets you describe a per-element transformation or predicate without declaring a named `smelt.define`.

Lambdas are a meta-world construct. They are evaluated entirely at compile time and never reach the database engine.

## Syntax: `fn x => body`

```
fn IDENT => EXPR
```

- `fn` is a reserved keyword that introduces the lambda.
- `IDENT` is the single parameter name, bound for use inside `EXPR`.
- `=>` is the lambda arrow (distinct from the `name => value` named-argument separator that appears outside `fn` bodies).
- `EXPR` is any meta-evaluable expression: a `smelt.<path>(...)` call, a HOF call, a pipe chain, a list literal, or an arithmetic/comparison expression involving the bound name.

**Example — double each element:**

```sql
-- examples/meta_hofs/models/pipe_rewrite.sql
-- fn c => c * 2 transforms every element of the list.
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- Engine sees the meta-evaluated result: SELECT 2, 4, 6
```

**Example — compose with `and_all`:**

```sql
-- reduce([true, false, true], and_all) reduces a Boolean list.
-- The lambda here is inside the filter:
SELECT
    reduce(
        filter([true, false, true], fn b => b),
        and_all
    )
```

## Single-argument only (v1)

Phase B supports exactly one lambda parameter. Multi-argument syntax (`fn (a, b) => body`) is reserved for Phase F. Writing it in Phase B emits `LambdaArityNotSupported`.

## Where lambdas are allowed

A lambda is only valid as a **positional argument to a HOF** (`map`, `filter`). Writing a lambda anywhere else emits `LambdaInForbiddenPosition` anchored at the `fn` keyword:

| Position | Allowed? |
|----------|----------|
| Second argument to `map(xs, fn x => ...)` | Yes |
| Second argument to `filter(xs, fn x => ...)` | Yes |
| List element `[fn x => x, ...]` | No — `LambdaInForbiddenPosition` |
| Named-argument value `p => fn x => x` | No — `LambdaInForbiddenPosition` |
| `smelt.define` parameter or return type | No — `Lambda<T, U>` is not a user-writable annotation |
| Top-level expression | No — `LambdaInForbiddenPosition` |

## Parameter scoping

Inside the lambda body, the bound name resolves **before any wider scope**. The resolution order (from `scoping.md`) is:

1. Lambda parameter — wins over everything inside the body.
2. `smelt.define` function parameters.
3. CTE columns visible at the reference site.
4. FROM-scope columns from `TableExpr` parameters.

A lambda parameter may shadow a same-named `smelt.define` parameter or column. The shadow is intentional (lexical scoping); the inner binding wins. To reach a shadowed outer name, assign it to an intermediate variable before the lambda, or use a qualified reference.

**Example — lambda parameter shadows outer binding:**

```sql
-- `c` inside the lambda refers to the element, not any outer `c`.
SELECT map([1, 2, 3], fn c => c + 10)
```

## What a lambda body can reference

| What | Allowed? |
|------|----------|
| The lambda's own parameter | Yes |
| Enclosing `smelt.define` parameters | Yes |
| Meta-only outer-scope values (`List<T>`, `smelt.config.var('x')` results) | Yes |
| SQL columns only available at Data-World runtime | No — `UnknownIdentifier` at the bare reference |

Lambdas capture the compile-time meta-world. Runtime SQL columns do not exist at meta-evaluation time and cannot be referenced inside a lambda body.

## LSP support

- **Hover** on the lambda parameter inside the body shows the parameter's bound type (supplied by the surrounding HOF's `T`).
- **Goto-definition** on the lambda parameter inside the body resolves to the parameter's binding occurrence in the lambda head (`fn` token).
- **Completion** inside a lambda body offers the bound parameter as the first identifier completion.

## Diagnostic codes

---

!!! warning "LambdaInForbiddenPosition"
    **When it fires:** A `fn x => body` lambda appears outside a HOF positional argument position.

    **Message:** `lambda is only valid as an argument to a higher-order function`

    **Fires at:** the `fn` keyword.

    **Example:**
    ```sql
    -- ← LambdaInForbiddenPosition: lambda is not a HOF argument here
    SELECT fn x => x + 1 FROM smelt.sources.raw.users
    ```

    **What to fix:** Move the lambda inside a call to `map` or `filter`. If you want to apply a transformation to every element of a list, use `map(xs, fn x => ...)`. If you need a named transformation, define it with `smelt.define`.

---

!!! warning "LambdaArityNotSupported"
    **When it fires:** A lambda with more than one parameter is written: `fn (a, b) => body`.

    **Message:** `multi-argument lambdas are not supported in v1; use a single parameter`

    **Fires at:** the parameter list.

    **Example:**
    ```sql
    -- ← LambdaArityNotSupported: (a, b) is multi-arg syntax, not supported in v1
    SELECT map([1, 2, 3], fn (a, b) => a + b)
    ```

    **What to fix:** Rewrite to use a single lambda parameter. If your transformation needs two values from the list, consider restructuring the source list or using `smelt.define` to accept both as regular parameters. Multi-argument lambdas are planned for Phase F.

---

!!! warning "LambdaResultTypeMismatch"
    **When it fires:** The lambda body's synthesised type is incompatible with what the surrounding HOF requires.

    **Message:** `{hof} requires lambda result {expected}; found {actual}`

    **Fires at:** the body expression.

    **Example:**
    ```sql
    -- filter requires Lambda<T, Boolean>; returning an integer instead
    -- ← LambdaResultTypeMismatch: filter requires Boolean; found INTEGER
    SELECT filter([1, 2, 3], fn c => c + 1)
    ```

    **What to fix:** Adjust the body expression to match the HOF's required return type. For `filter`, the body must evaluate to a `Boolean` predicate (e.g. `c > 0`, `c IS NOT NULL`). For `map`, the body may return any type, but all elements must share the same result type.
