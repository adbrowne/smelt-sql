---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.segment,
    b.campaign_id,
    c.region,
    c.product_id
FROM smelt.ref('py_l2_410') a
INNER JOIN smelt.ref('sql_l2_13') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_217') c ON a.user_id = c.user_id
