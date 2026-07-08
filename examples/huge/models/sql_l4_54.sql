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
    region,
    browser,
    ROW_NUMBER() OVER (PARTITION BY region ORDER BY created_at) AS win_val
FROM smelt.sql_l3_191
