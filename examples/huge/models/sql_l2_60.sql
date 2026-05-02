---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.rating,
    a.ip_address,
    b.email_domain
FROM smelt.models.sql_l1_211 a
LEFT JOIN smelt.models.sql_l1_211 b ON a.user_id = b.user_id

