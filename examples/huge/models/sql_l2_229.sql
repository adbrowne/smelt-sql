---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.email_domain,
    b.region,
    c.discount,
    c.plan_type
FROM smelt.ref('sql_l1_94') a
INNER JOIN smelt.ref('sql_l1_247') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_208') c ON a.user_id = c.user_id
