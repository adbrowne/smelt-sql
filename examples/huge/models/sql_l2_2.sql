---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    AVG(price) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1
FROM smelt.models.sql_l1_31
GROUP BY device_type

