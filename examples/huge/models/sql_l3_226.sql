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
    a.campaign_id,
    a.ip_address,
    b.platform
FROM smelt.sql_l2_220 a
LEFT JOIN smelt.sql_l2_172 b ON a.user_id = b.user_id
