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
    a.order_id,
    b.cohort_date
FROM smelt.sql_l1_211 a
INNER JOIN smelt.sql_l1_120 b ON a.user_id = b.user_id

