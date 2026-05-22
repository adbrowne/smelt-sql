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
    a.platform,
    b.device_type,
    c.tier,
    c.os_name
FROM smelt.sql_l1_55 a
INNER JOIN smelt.sql_l1_67 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_55 c ON a.user_id = c.user_id
