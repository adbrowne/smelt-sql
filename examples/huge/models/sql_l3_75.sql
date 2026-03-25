---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cohort_date,
    b.email_domain,
    c.ip_address,
    c.updated_at
FROM smelt.ref('sql_l2_224') a
INNER JOIN smelt.ref('sql_l2_223') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_39') c ON a.user_id = c.user_id
