-- order_totals: aggregates order revenue by day.
--
-- This model calls smelt.fn.enriched_order but only projects order-side
-- columns (order_id, total).  Because the enriched_order function declares
-- a 1:1 join against dim_customer with cardinality: "1:1" in its joins:
-- frontmatter, the EliminateUnusedLeftJoin planner rule can safely elide
-- the LEFT JOIN — no customer columns appear in the SELECT list.
SELECT
    order_id,
    total
FROM smelt.fn.enriched_order(smelt.ref('orders'))
