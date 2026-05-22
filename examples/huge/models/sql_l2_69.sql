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
    a.category,
    a.page_path,
    b.profit
FROM smelt.sql_l1_105 a
LEFT JOIN smelt.sql_l1_37 b ON a.user_id = b.user_id
