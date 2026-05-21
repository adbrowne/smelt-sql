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
    a.rating,
    a.ip_address,
    b.email_domain
FROM smelt.sql_l1_211 a
LEFT JOIN smelt.sql_l1_211 b ON a.user_id = b.user_id

