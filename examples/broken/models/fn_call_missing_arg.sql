-- Phase 6 broken fixture: the required `b` parameter is omitted. The call
-- site must emit exactly one `MissingArgument` diagnostic anchored at the
-- call-path span.
smelt.define takes_two(a: Expr<Integer>, b: Expr<Integer>) -> Expr<Integer> AS (a + b)

SELECT smelt.fn.takes_two(1) AS r
