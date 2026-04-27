# Upgrading a smelt function from Tier 1 to Tier 2

This document explains what changes when you add type annotations to a
`smelt.define` function and how to resolve the diagnostics that may appear at
existing call sites.

## What are Tier 1 and Tier 2?

**Tier 1** functions have no parameter type annotations:

```sql
smelt.define safe_divide(numerator, denominator) AS (
  CAST(numerator AS DOUBLE) / NULLIF(denominator, 0)
)
```

The body is expanded at every call site with the caller's concrete argument
types substituted in.  Type errors are only reported at call sites where the
concrete types actually cause a problem — so a partially-wrong body may pass
silently if callers happen to pass compatible types.

**Tier 2** functions annotate every parameter:

```sql
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) AS (
  CAST(numerator AS DOUBLE) / NULLIF(denominator, 0)
)
```

The body is checked once at definition time with the declared parameter types.
Errors are reported against the function body, not against each call site
separately.  A Tier 3 function additionally annotates the return type
(`-> Expr<Double>`), enabling a definition-time return-type check.

## What breaks when you add annotations?

Callers whose argument types do not satisfy the newly-declared constraint will
now get an `ArgTypeMismatch` diagnostic.  Previously those calls were silently
expanded; now the compiler rejects them.

For example, if a caller passes a `Text` value to `safe_divide`:

```sql
-- model that calls safe_divide with a text column
SELECT smelt.fn.safe_divide(revenue_text, 100) AS rate
```

Before the annotation, smelt expanded the body with `numerator = Text` and the
resulting SQL was accepted by the backend.  After the `Expr<Numeric>` annotation
the compiler emits:

```
ArgTypeMismatch: Argument `revenue_text` has type `TEXT`, which does not
satisfy parameter `numerator: Expr<Numeric>` of `safe_divide`
```

## How to fix

Two options:

1. **Widen the annotation** — if you want the function to legitimately accept
   wider types, change `Expr<Numeric>` to `Expr<Any>` or an appropriate
   constraint.  This preserves backward compatibility for callers.

2. **Fix the call site** — if the caller is genuinely wrong (a `Text` column
   should never be divided), add an explicit `CAST` at the call site:

   ```sql
   SELECT smelt.fn.safe_divide(CAST(revenue_text AS DOUBLE), 100) AS rate
   ```

## Concrete before/after: `safe_divide`

**Before (Tier 1)**:

```sql
smelt.define safe_divide(numerator, denominator) AS (
  CAST(numerator AS DOUBLE) / NULLIF(denominator, 0)
)
```

- Body checked only at call sites.
- A call like `smelt.fn.safe_divide('bad', 1)` passes silently.

**After (Tier 2 / Tier 3)**:

```sql
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>)
  -> Expr<Double> AS (
  CAST(numerator AS DOUBLE) / NULLIF(denominator, 0)
)
```

- Body checked once at definition time.
- `smelt.fn.safe_divide('bad', 1)` now raises `ArgTypeMismatch` immediately.
- The declared `-> Expr<Double>` return type is verified against the body.
- Existing callers that pass numeric columns continue to work unchanged.

## Diagnostics reference

| Diagnostic | Meaning | Fix |
|---|---|---|
| `ArgTypeMismatch` | Argument type doesn't satisfy the parameter constraint | Widen annotation or CAST at call site |
| `ReturnTypeMismatch` | Body evaluates to a type incompatible with `-> Expr<T>` | Adjust the body or the declared return type |
| `FunctionBodyTypeMismatch` | A subexpression in the body has an unexpected type | Fix the body expression |
