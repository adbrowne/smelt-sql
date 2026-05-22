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
    a.event_time,
    b.country,
    c.cohort_date,
    c.email_domain
FROM smelt.sql_l2_202 a
INNER JOIN smelt.sql_l2_185 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_202 c ON a.user_id = c.user_id
