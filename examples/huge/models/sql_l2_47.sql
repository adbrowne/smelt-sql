---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    cohort_date,
    browser,
    transaction_id
FROM smelt.sql_l1_53
WHERE status = 'active'
