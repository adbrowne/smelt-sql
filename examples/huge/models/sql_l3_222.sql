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
    a.campaign_id,
    b.rating
FROM smelt.sql_l2_118 a
LEFT JOIN smelt.sql_l2_127 b ON a.user_id = b.user_id

