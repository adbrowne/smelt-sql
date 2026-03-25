---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.email_domain,
    a.cohort_date,
    b.order_id
FROM smelt.ref('sql_l1_190') a
LEFT JOIN smelt.ref('sql_l1_190') b ON a.user_id = b.user_id
