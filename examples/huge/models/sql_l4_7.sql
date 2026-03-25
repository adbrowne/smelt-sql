---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    a.discount,
    b.campaign_id
FROM smelt.ref('sql_l3_94') a
INNER JOIN smelt.ref('py_l3_262') b ON a.user_id = b.user_id
