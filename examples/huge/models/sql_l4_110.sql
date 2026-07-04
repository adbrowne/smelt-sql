---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    is_active,
    page_path,
    region
FROM smelt.sql_l3_60
WHERE quantity > 0
