---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.amount,
    a.event_type,
    b.cohort_date
FROM smelt.ref('sql_l1_195') a
LEFT JOIN smelt.ref('sql_l1_171') b ON a.user_id = b.user_id
