-- Phase 15 fixture: a minimal `orders` model exposing a {revenue, cost}
-- schema so `smelt.fn.add_margin(smelt.ref('orders'))` has a valid
-- `TableExpr` argument. The body SELECT is intentionally literal — the
-- point is the column-typed row shape, not the values.
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS DECIMAL(18, 2)) AS revenue,
  CAST(NULL AS DECIMAL(18, 2)) AS cost
