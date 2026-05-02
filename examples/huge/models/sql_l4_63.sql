---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.product_id,
    b.campaign_id,
    c.event_type,
    c.ip_address
FROM smelt.models.sql_l3_4 a
INNER JOIN smelt.models.sql_l3_69 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l3_170 c ON a.user_id = c.user_id

