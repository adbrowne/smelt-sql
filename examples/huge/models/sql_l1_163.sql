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
    plan_type,
    cohort_date,
    LAG(amount, 1) OVER (PARTITION BY plan_type ORDER BY created_at) AS win_val
FROM smelt.page_views

