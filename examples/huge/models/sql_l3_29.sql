---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    b.country,
    c.cohort_date,
    c.email_domain
FROM smelt.models.sql_l2_202 a
INNER JOIN smelt.models.sql_l2_185 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l2_202 c ON a.user_id = c.user_id

