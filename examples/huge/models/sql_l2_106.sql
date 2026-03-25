---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.email_domain,
    a.product_id,
    b.channel
FROM smelt.ref('sql_l1_240') a
INNER JOIN smelt.ref('sql_l1_240') b ON a.user_id = b.user_id
