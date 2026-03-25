---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    b.country,
    c.cohort_date,
    c.email_domain
FROM smelt.ref('py_l2_359') a
INNER JOIN smelt.ref('py_l2_295') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_215') c ON a.user_id = c.user_id
