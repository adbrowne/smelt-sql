---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    category,
    os_name,
    product_id
FROM smelt.sql_l1_68
WHERE created_at >= '2024-01-01'
