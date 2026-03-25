---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    region,
    product_id,
    RANK() OVER (PARTITION BY region ORDER BY created_at) AS win_val
FROM smelt.ref('users')
