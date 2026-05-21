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
    a.created_at,
    a.campaign_id,
    b.referrer
FROM smelt.sql_l1_23 a
INNER JOIN smelt.sql_l1_59 b ON a.user_id = b.user_id

