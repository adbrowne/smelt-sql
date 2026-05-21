---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    product_id,
    plan_type,
    status
FROM smelt.orders
WHERE created_at >= '2024-01-01'

