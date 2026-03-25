---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    status,
    rating,
    tier
FROM smelt.ref('sql_l3_176')
WHERE event_type = 'purchase'
