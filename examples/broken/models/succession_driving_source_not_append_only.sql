---
materialization: table
refresh: incremental
---
-- SuccessionDrivingSourceNotAppendOnly: `maintenance_orders` declares
-- mutation_profile: append_only but no `timeseries:` block, so it has no
-- clock (`docs/specs/incremental_shapes.md` §"Run shape and late events").
SELECT
    customer_id,
    order_date,
    LEAD(order_date) OVER (PARTITION BY customer_id ORDER BY order_date) AS next_order_date
FROM smelt.sources.maintenance_orders
