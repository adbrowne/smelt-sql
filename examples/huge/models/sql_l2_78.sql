---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    a.is_verified,
    b.cohort_date
FROM smelt.ref('py_l1_271') a
LEFT JOIN smelt.ref('sql_l1_211') b ON a.user_id = b.user_id
