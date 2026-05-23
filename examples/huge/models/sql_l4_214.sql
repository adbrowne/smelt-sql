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
    a.category,
    b.tier,
    c.duration_seconds,
    c.email_domain
FROM smelt.sql_l3_101 a
INNER JOIN smelt.sql_l3_133 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_167 c ON a.user_id = c.user_id
