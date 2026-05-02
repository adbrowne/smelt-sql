---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.segment,
    b.campaign_id,
    c.region,
    c.product_id
FROM smelt.models.sql_l2_123 a
INNER JOIN smelt.models.sql_l2_169 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l2_241 c ON a.user_id = c.user_id

