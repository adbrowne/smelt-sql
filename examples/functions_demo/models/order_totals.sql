-- order_totals: aggregates order revenue by day.
--
-- This model calls smelt.fn.enriched_order but only projects order-side
-- columns (order_id, total).  The enriched_order function LEFT JOINs
-- dim_customer to surface its customer_name / customer_tier columns
-- (Phase 45 lets the body reference those columns directly through the
-- join alias). Because the function declares cardinality: "1:1" in its
-- joins: frontmatter, the EliminateUnusedLeftJoin planner rule can
-- safely elide the LEFT JOIN at this call site — no customer columns
-- appear in the SELECT list.
SELECT
    order_id,
    total
FROM smelt.fn.enriched_order(smelt.ref('orders'))
