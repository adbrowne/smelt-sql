---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.segment,
    a.cohort_date,
    b.category
FROM smelt.ref('py_l1_278') a
INNER JOIN smelt.ref('sql_l1_194') b ON a.user_id = b.user_id
