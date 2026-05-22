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
    a.quantity,
    b.campaign_id,
    c.discount,
    c.profit
FROM smelt.sql_l2_174 a
INNER JOIN smelt.sql_l2_156 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_174 c ON a.user_id = c.user_id
