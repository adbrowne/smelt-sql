-- Phase 46 fixture: pass a CTE reference into a TableExpr-parameterised
-- function. Demonstrates that `smelt.fn.add_margin(x)` resolves the
-- CTE's schema correctly without the caller having to round-trip
-- through `smelt.ref()`.
WITH x AS (
  SELECT
    CAST(100 AS DECIMAL(18, 2)) AS revenue,
    CAST(30 AS DECIMAL(18, 2)) AS cost
)
SELECT margin
FROM smelt.fn.add_margin(x)
