---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cost,
    b.email_domain,
    c.referrer,
    c.cohort_date
FROM smelt.models.sql_l1_74 a
INNER JOIN smelt.models.sql_l1_215 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l1_74 c ON a.user_id = c.user_id

