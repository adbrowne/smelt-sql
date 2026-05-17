# Pipe Operator `|>`

The meta-language provides the **pipe operator `|>`**, which lets you write a chain of [HOF](hofs.md) calls in left-to-right reading order instead of nesting them inside-out. The pipe operator works particularly well with [lambdas](lambdas.md): `xs |> filter(fn x => …) |> map(fn x => …)` reads as a natural left-to-right pipeline.

The pipe operator is meta-world only: both sides evaluate at compile time. No `|>` expression ever reaches the database engine.

## Syntax and semantics

```
LHS |> f(args...)   ≡   f(LHS, args...)
```

`|>` inserts the left-hand side as the **first argument** to the call on the right-hand side. Everything else about the call — its type, its result, its diagnostic anchoring — is identical to writing the un-piped form directly. Pipe is purely syntactic sugar and is desugared before type-checking.

**Example — single pipe:**

```sql
-- [1, 2, 3] |> filter(fn c => c > 0)
-- desugars to: filter([1, 2, 3], fn c => c > 0)
SELECT [1, 2, 3] |> filter(fn c => c > 0)
```

**Example — chained pipes (from `examples/meta_hofs/`):**

```sql
-- examples/meta_hofs/models/pipe_rewrite.sql
-- [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- desugars to: map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
```

## Associativity and precedence

`|>` is **left-associative** with the **lowest precedence** among meta-language operators:

```
a |> b(p) |> c(q)   parses as   (a |> b(p)) |> c(q)
                    desugars to  c(b(a, p), q)
```

Pipe has lower precedence than function calls, spread (`...`), and field access. It has higher precedence than statement separators (`;`) — pipe never crosses a statement boundary.

## Pipe is meta-world only

`|>` evaluates only in the meta world. Both the left-hand expression and the right-hand call are compile-time meta-language constructs. Attempting to use pipe inside a Data-World position (for example, inside a `WHERE` predicate) emits `PipeInDataPosition`.

This is a deliberate scope cut. Extending `|>` into Data-World SQL (pipe-SQL) is a separate design that affects the planner and query executor, not just the meta-language.

## The RHS must be a call

The right-hand side of `|>` must syntactically be a function call — one of:
- A HOF call: `map(args)`, `filter(args)`, `reduce(args)`
- A `smelt.<path>(args)` call
- Any other function call: `f(args)`

A non-call right-hand side (`x |> y`, `x |> 3 + 4`) emits `PipeRhsNotCall` at the RHS span.

## LSP support

- **Hover** on a pipe expression shows the result type of the equivalent un-piped call — identical to hovering on the desugared form.
- Pipe expressions participate in the same frame-stack tracing as their un-piped equivalents. A diagnostic that fires inside a piped HOF chain carries the same frames as the un-piped version.

## Diagnostic codes

---

!!! warning "PipeRhsNotCall"
    **When it fires:** The right-hand side of `|>` is not a function call expression.

    **Message:** `pipe right-hand side must be a function call`

    **Fires at:** the RHS span.

    **Example:**
    ```sql
    -- ← PipeRhsNotCall: 'some_name' is not a call expression
    SELECT [1, 2, 3] |> some_name
    ```

    **What to fix:** The right-hand side of `|>` must be a call: `f(args)`, `map(fn x => ...)`, `smelt.my_fn(args)`, etc. If you mean to pass the left-hand value as the only argument, write `f(left)` without pipe, or add empty parentheses: `f()` with `LHS |> f()`.

---

!!! warning "PipeInDataPosition"
    **When it fires:** A pipe expression (`LHS |> f(...)`) appears in a Data-World grammar position — for example, inside a `WHERE` predicate or as the right-hand side of a SQL binary operator.

    **Message:** `|> is meta-only; use SQL composition in this position`

    **Fires at:** the `|>` token.

    **Example:**
    ```sql
    SELECT id
    FROM smelt.sources.raw.users
    -- ← PipeInDataPosition: |> is meta-only; cannot use it in WHERE
    WHERE active = true AND [1, 2, 3] |> filter(fn c => c > 0)
    ```

    **What to fix:** Pipe is only valid in meta-world positions (where the result is a list or a meta-evaluable expression). Move the pipe chain to a compile-time context — for example, bind the result to a `smelt.define` parameter or use it where a `List<T>` is expected. For WHERE-clause predicate composition from a list, use the `and_all` or `or_any` reducer directly.
