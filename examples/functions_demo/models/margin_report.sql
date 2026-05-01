-- Phase 25 upgrade: call the Tier 3 `margin_tier3` function (fully annotated
-- parameters + declared return type) instead of the TableExpr-based `add_margin`.
-- `margin_tier3(revenue, cost)` takes two Expr<Numeric> args and returns
-- Expr<Double> — the revenue and cost columns from `orders` satisfy Numeric.
SELECT order_id, smelt.functions.margin_tier3(revenue, cost) AS margin
FROM smelt.models.orders

