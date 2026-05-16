# Ternary

The meta-language provides a **compile-time ternary expression** — `if cond then a else b` — for branching on Boolean values that are known at build time. The condition and both branches are fully type-checked; exactly one branch is evaluated at compile time.

Ternary expressions are a meta-world construct. The database engine never sees `if`/`then`/`else`; it sees only the chosen branch's compiled SQL fragment.

## Syntax: `if cond then a else b`

```
if COND then THEN_EXPR else ELSE_EXPR
```

- `if`, `then`, `else` are reserved keywords.
- `COND` must synthesise to a `Boolean` meta value.
- `THEN_EXPR` and `ELSE_EXPR` are arbitrary meta-evaluable expressions whose types must unify under the least-upper-bound (LUB) rules.
- The ternary's synthesised type is the LUB of the two branch types.

**Example — select a SQL expression based on a compile-time flag:**

```sql
-- examples/meta_polish/models/ternary_literal_cond.sql
-- if true then 'strict' else 'permissive' resolves at compile time.
-- The engine sees only: SELECT 'strict'
SELECT if true then 'strict' else 'permissive'
```

## Where ternary is allowed

Ternary expressions are valid anywhere a meta-evaluable expression is allowed. They are **not** valid in Data-World grammar positions (inside a plain SQL `WHERE` clause or `SELECT` item that accepts only SQL expressions). A ternary in a data position emits `TernaryInDataPosition`; use SQL `CASE WHEN … THEN … ELSE … END` instead.

## Evaluation rules

### Compile-time short-circuit evaluation

The ternary evaluates at compile time, choosing exactly one branch:

1. `COND` is evaluated exactly once.
2. If `COND` is `TRUE`, `THEN_EXPR` is evaluated and `ELSE_EXPR` is **not** evaluated.
3. If `COND` is `FALSE`, `ELSE_EXPR` is evaluated and `THEN_EXPR` is **not** evaluated.

**Diagnostics from evaluation** (such as `MapGetMissingKey` or `ConfigVarNotFound`) are suppressed for the unreached branch. **Type-checking diagnostics** (such as ill-typed subexpressions) are still emitted for both branches regardless of which is reached.

### Branch type unification

`THEN_EXPR` and `ELSE_EXPR` are type-checked independently. Their synthesised types must unify under the same LUB rules used for list literals — compatible numeric types are promoted, and sub-sort assignability applies. The ternary's synthesised type is the LUB.

If the two branch types cannot be unified, `TernaryBranchTypeMismatch` is emitted.

### `Unknown` propagation

If `COND` synthesises to `Unknown` (for example because it depends on a missing config variable), neither branch is evaluated and the ternary's value is `Unknown`. Both branches are still type-checked.

### No scope

The ternary introduces no new bindings. `COND`, `THEN_EXPR`, and `ELSE_EXPR` all resolve names against the surrounding context's scope unchanged.

### Determinism and termination

The ternary is a pure value expression. Given the same workspace state, the condition evaluates to the same value and the same branch is chosen. The compile-time cost is one Boolean check plus the cost of evaluating the chosen branch.

## Precedence and associativity

The ternary has **lower precedence** than the pipe operator (`|>`). This means:

```sql
-- Parses as: (xs |> map(fn x => x)) if cond then a else b
xs |> map(fn x => x) if cond then a else b
```

The `if` begins a fresh ternary with the entire preceding expression as `COND`. To pipe a ternary as a function argument, parenthesise it:

```sql
-- Pipe a ternary value into a function:
reduce(
    if use_strict then strict_list else permissive_list,
    and_all
)

-- Or parenthesise to pass a ternary as a HOF argument:
map(xs, fn x => (if x > 0 then x else 0))
```

Ternary chains are **right-associative**. The `else` clause extends as far right as possible, consuming a trailing ternary:

```sql
-- Parses as: if c1 then a else (if c2 then b else c)
if c1 then a else if c2 then b else c
```

This is the natural `else if` chaining form. No special `elsif` keyword is needed.

## Defaulting pattern with `m.has(k)`

The short-circuit evaluation rule makes the ternary the idiomatic way to provide a default value when accessing a map key that may be absent:

```sql
-- Guard m.get(k) with m.has(k) to avoid MapGetMissingKey on the else branch.
-- If m.has('env') is FALSE, m.get('env') is in the unreached branch and its
-- MapGetMissingKey diagnostic is suppressed.
SELECT if m.has('env') then m.get('env') else 'production'
```

This pattern works because `MapGetMissingKey` is an *evaluation-time* diagnostic — it fires only when `m.get(k)` is actually evaluated. With short-circuit evaluation, an absent key never reaches evaluation in the guarded branch.

## Keyword reservation

`if`, `then`, and `else` are reserved at the meta-namespace level. A `smelt.define` function, a `smelt.record` declaration, or a lambda parameter named one of these keywords emits `TernaryKeywordShadowed`.

```sql
-- ← TernaryKeywordShadowed: 'else' is a reserved meta-language keyword
smelt.define else(xs: List<Text>) -> Text ...
```

## LSP support

- **Hover** on the `if` keyword shows the full inferred ternary signature — `if cond:{COND_type} then a:{THEN_type} else b:{ELSE_type} -> {LUB_type}` — with the LUB resolved when both branches synthesise.
- **Hover** on a `then` keyword shows the `THEN_EXPR` branch's synthesised type.
- **Hover** on an `else` keyword shows the `ELSE_EXPR` branch's synthesised type.
- **Goto-definition** on `if`/`then`/`else` resolves to this reference page by URL hint when the LSP client supports external links; otherwise a graceful no-op.
- **Completion** at the start of a meta-evaluable position offers `if` as a snippet expanding to `if $cond then $then_expr else $else_expr`.

## Diagnostic codes

---

<a id="ternaryconditionnotboolean"></a>
!!! warning "TernaryConditionNotBoolean"
    **When it fires:** The `COND` expression in `if COND then … else …` synthesises to a type that is not assignable to `Boolean`.

    **Message:** `ternary condition expects Boolean; found {actual}`

    **Fires at:** the `COND` expression span.

    **Example:**
    ```sql
    -- ← TernaryConditionNotBoolean: TEXT is not assignable to Boolean
    SELECT if 'yes' then 1 else 0
    ```

    **What to fix:** The condition must evaluate to a `Boolean` meta value. Use a comparison (`x > 0`, `m.has('key')`, `smelt.config.var('flag') = 'true'`) rather than a string or integer. The meta-language has no Boolean coercion — there is no truthy/falsy interpretation of non-Boolean values.

---

<a id="ternarybranchtypemismatch"></a>
!!! warning "TernaryBranchTypeMismatch"
    **When it fires:** The `THEN_EXPR` and `ELSE_EXPR` branches synthesise to types that do not unify under the LUB rules.

    **Message:** `ternary branches have incompatible types: {then_type} vs {else_type}`

    **Fires at:** the `else` keyword (the second branch boundary).

    **Example:**
    ```sql
    -- ← TernaryBranchTypeMismatch: Expr<INTEGER> vs Expr<TEXT>
    SELECT if true then 42 else 'hello'
    ```

    **What to fix:** Ensure both branches produce values of compatible types. Compatible numeric types (`INTEGER`, `BIGINT`, `DECIMAL`) are promoted automatically. Incompatible sorts (a number branch vs a text branch, or a `List<T>` branch vs an `Expr<U>` branch) require explicit handling — restructure the logic so both paths return the same kind of value.

---

<a id="ternarykeywordshadowed"></a>
!!! warning "TernaryKeywordShadowed"
    **When it fires:** A `smelt.define` function, `smelt.record` declaration, or lambda parameter is declared with the name `if`, `then`, or `else`.

    **Message:** `{name} is a reserved meta-language keyword`

    **Fires at:** the declaration's name token.

    **Example:**
    ```sql
    -- ← TernaryKeywordShadowed: 'then' is a reserved meta-language keyword
    smelt.define then(xs: List<Text>) -> Text ...
    ```

    **What to fix:** Choose a different name for the declaration. The three keywords `if`, `then`, and `else` are reserved at the meta-namespace level and cannot be used as identifiers anywhere in smelt.

---

<a id="ternaryindataposition"></a>
!!! warning "TernaryInDataPosition"
    **When it fires:** A `if … then … else …` ternary expression appears in a Data-World grammar position that does not admit meta evaluation.

    **Message:** `if-then-else is meta-only; use SQL CASE WHEN in this position`

    **Fires at:** the `if` keyword.

    **Example:**
    ```sql
    SELECT id
    FROM smelt.sources.raw.users
    -- ← TernaryInDataPosition: ternary is meta-only in WHERE
    WHERE if is_admin then TRUE else FALSE
    ```

    **What to fix:** Replace the ternary with a SQL `CASE WHEN … THEN … ELSE … END` expression for Data-World conditional logic. Use a meta ternary only in positions that accept meta expressions — for example, inside a HOF lambda body, a `smelt.define` body, or a meta value argument.

---

<a id="ternarydanglingthen"></a>
!!! warning "TernaryDanglingThen"
    **When it fires:** A `then` keyword appears outside any in-progress ternary's `COND` slot — for example after the ternary has already consumed its `then` clause, or with no preceding `if`.

    **Message:** `unexpected 'then' keyword outside of 'if ... then ...' form`

    **Fires at:** the stray `then` token.

    **Example:**
    ```sql
    -- ← TernaryDanglingThen: 'then' without a preceding 'if'
    SELECT 'value' then 'other'
    ```

    **What to fix:** Check the ternary structure. Either add the missing `if COND` prefix, or remove the stray `then`. The right-associative chaining rule means `else if cond then …` is valid; a bare `then` in any other position is always an error.

---

<a id="ternarydanglingelse"></a>
!!! warning "TernaryDanglingElse"
    **When it fires:** An `else` keyword appears outside any in-progress ternary's `THEN_EXPR` slot — for example with no preceding `if … then`, or after the ternary has already consumed its `else` clause.

    **Message:** `unexpected 'else' keyword outside of '... then ... else' form`

    **Fires at:** the stray `else` token.

    **Example:**
    ```sql
    -- ← TernaryDanglingElse: 'else' without a preceding 'if ... then'
    SELECT if true then 1 else 2 else 3
    ```

    **What to fix:** Remove the extra `else` clause, or restructure the ternary chain. The right-associative rule means the first `else` already consumed the trailing `if c then b else c` sub-expression; a second `else` at the same level is always an error.
