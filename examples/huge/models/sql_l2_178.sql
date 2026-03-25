---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cost,
    b.email_domain,
    c.referrer,
    c.cohort_date
FROM smelt.ref('py_l1_394') a
INNER JOIN smelt.ref('py_l1_335') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l1_394') c ON a.user_id = c.user_id
