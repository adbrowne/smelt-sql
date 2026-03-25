---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    b.channel,
    c.is_active,
    c.cohort_date
FROM smelt.ref('py_l3_436') a
INNER JOIN smelt.ref('py_l3_436') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_436') c ON a.user_id = c.user_id
