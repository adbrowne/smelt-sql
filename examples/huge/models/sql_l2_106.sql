---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.email_domain,
    a.product_id,
    b.channel
FROM smelt.models.sql_l1_1 a
INNER JOIN smelt.models.sql_l1_218 b ON a.user_id = b.user_id

