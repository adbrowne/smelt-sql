-- Phase 25: Tier 2 function defined and called with wrong argument type.
-- `mul_typed_local` expects Expr<Integer> for both `x` and `y`, but the
-- literal 'hello' is Text. Expected: ArgTypeMismatch at the call site.
smelt.define mul_typed_local(x: Expr<Integer>, y: Expr<Integer>) AS (x * y)

SELECT smelt.fn.mul_typed_local('hello', 1) AS r
