---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.rating,
    a.ip_address,
    b.email_domain
FROM smelt.ref('sql_l1_226') a
LEFT JOIN smelt.ref('sql_l1_226') b ON a.user_id = b.user_id
