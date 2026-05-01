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
    a.cohort_date,
    b.order_id
FROM smelt.models.sql_l1_116 a
LEFT JOIN smelt.models.sql_l1_19 b ON a.user_id = b.user_id

