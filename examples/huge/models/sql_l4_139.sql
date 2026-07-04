---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.status,
    a.page_path,
    b.is_verified
FROM smelt.sql_l3_82 a
INNER JOIN smelt.sql_l3_82 b ON a.user_id = b.user_id
