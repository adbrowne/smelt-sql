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
    a.product_id,
    b.email_domain,
    c.os_name,
    c.duration_seconds
FROM smelt.sql_l3_119 a
INNER JOIN smelt.sql_l3_119 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_119 c ON a.user_id = c.user_id
