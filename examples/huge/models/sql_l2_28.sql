---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    created_at,
    cost,
    event_date
FROM smelt.ref('sql_l1_109')
WHERE score >= 50
