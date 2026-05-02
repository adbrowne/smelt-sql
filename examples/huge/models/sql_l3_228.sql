---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.channel,
    b.referrer,
    c.browser,
    c.product_id
FROM smelt.models.sql_l2_9 a
INNER JOIN smelt.models.sql_l2_46 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l2_36 c ON a.user_id = c.user_id

