---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: hour
---
-- Declares `granularity: hour` but the model's own `order_date` projection
-- only truncates to `day` — a narrowing declaration (finer than what the
-- grouping actually derives), refused with `MaintenanceGranularityMismatch`
-- (`docs/specs/incremental_models.md` §Design "Grain is declared" /
-- "Widen-never-narrow").
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.customer_id) AS total_customers
FROM smelt.sources.maintenance_orders o
GROUP BY 1
