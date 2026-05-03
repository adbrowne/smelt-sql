---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.campaign_id,
    b.browser,
    c.cost,
    c.price
FROM smelt.sql_l2_12 a
INNER JOIN smelt.sql_l2_202 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_12 c ON a.user_id = c.user_id

