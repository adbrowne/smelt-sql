---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    a.channel,
    b.product_id
FROM smelt.ref('sql_l1_97') a
INNER JOIN smelt.ref('sql_l1_55') b ON a.user_id = b.user_id
