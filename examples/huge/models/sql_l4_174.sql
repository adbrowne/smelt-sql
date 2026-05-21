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
    event_date,
    amount,
    session_id,
    product_id
FROM smelt.sql_l3_187
WHERE status = 'active'

