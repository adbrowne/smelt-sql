---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.segment,
    a.cohort_date,
    b.amount
FROM smelt.ref('py_l2_442') a
INNER JOIN smelt.ref('sql_l2_37') b ON a.user_id = b.user_id
