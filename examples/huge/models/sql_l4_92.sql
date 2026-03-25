---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    region,
    category,
    is_verified
FROM smelt.ref('sql_l3_176')
WHERE score >= 50
