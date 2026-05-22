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
    a.os_name,
    b.profit,
    c.user_id,
    c.page_path
FROM smelt.sql_l2_167 a
INNER JOIN smelt.sql_l2_45 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_247 c ON a.user_id = c.user_id
