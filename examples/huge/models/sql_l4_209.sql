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
    country,
    ROW_NUMBER() OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_115')
