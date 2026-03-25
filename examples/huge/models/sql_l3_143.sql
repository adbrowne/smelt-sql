---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    referrer,
    country,
    price
FROM smelt.ref('sql_l2_102')
WHERE platform = 'web'
