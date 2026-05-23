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
    a.cohort_date,
    b.email_domain,
    c.ip_address,
    c.updated_at
FROM smelt.sql_l2_122 a
INNER JOIN smelt.sql_l2_132 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_196 c ON a.user_id = c.user_id
