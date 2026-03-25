---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    b.campaign_id,
    c.event_type,
    c.ip_address
FROM smelt.ref('py_l3_276') a
INNER JOIN smelt.ref('py_l3_415') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_403') c ON a.user_id = c.user_id
