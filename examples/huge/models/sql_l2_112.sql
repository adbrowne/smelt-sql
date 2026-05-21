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
    duration_seconds,
    transaction_id,
    rating,
    product_id
FROM smelt.sql_l1_117
WHERE status = 'active'

