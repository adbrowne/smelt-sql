-- Phase 34 fixture: dimension table for the join-elimination demo.
-- `enriched_order` LEFT JOINs this table on `customer_id`.
-- `order_totals` projects no dim_customer columns, so the planner
-- can elide the join via EliminateUnusedLeftJoin.
SELECT
  CAST(NULL AS VARCHAR) AS customer_id,
  CAST(NULL AS VARCHAR) AS customer_name,
  CAST(NULL AS VARCHAR) AS customer_tier

