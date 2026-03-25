---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.order_id,
    a.cohort_date,
    b.event_time
FROM smelt.ref('sql_l3_63') a
LEFT JOIN smelt.ref('sql_l3_238') b ON a.user_id = b.user_id
