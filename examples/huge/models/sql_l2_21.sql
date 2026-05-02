---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.platform,
    a.channel,
    b.product_id
FROM smelt.models.sql_l1_6 a
INNER JOIN smelt.models.sql_l1_6 b ON a.user_id = b.user_id

