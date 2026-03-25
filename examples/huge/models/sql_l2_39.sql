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
    rating,
    cohort_date,
    profit
FROM smelt.ref('sql_l1_161')
WHERE created_at >= '2024-01-01'
