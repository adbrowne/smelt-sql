---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.order_id,
    a.cohort_date,
    b.event_time
FROM smelt.ref('py_l3_437') a
LEFT JOIN smelt.ref('py_l3_437') b ON a.user_id = b.user_id
