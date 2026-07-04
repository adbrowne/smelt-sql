---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    category,
    page_path,
    created_at
FROM smelt.sql_l2_54
WHERE platform = 'web'
