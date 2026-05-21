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
    a.device_type,
    b.tier,
    c.browser,
    c.campaign_id
FROM smelt.sql_l1_157 a
INNER JOIN smelt.sql_l1_220 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_157 c ON a.user_id = c.user_id

