---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    page_path,
    LAG(amount, 1) OVER (PARTITION BY cohort_date ORDER BY created_at) AS win_val
FROM smelt.transactions
