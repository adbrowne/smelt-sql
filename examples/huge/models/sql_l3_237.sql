---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    country,
    profit,
    category
FROM smelt.ref('sql_l2_102')
WHERE quantity > 0
