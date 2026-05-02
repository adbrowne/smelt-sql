-- Phase 26: Tier 2 body calling a Tier 1 callee that has a type error.
-- `broken_helper` body `x + 'text'` fails under Integer arg from `tier2_wrapper`.
-- Expected: FunctionBodyTypeMismatch at the Tier 2 definition site.
smelt.define broken_helper(x) AS (x + 'text')

smelt.define tier2_wrapper(n: Expr<Integer>) AS (
    smelt.models.broken_helper(n)
)
