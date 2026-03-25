---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.campaign_id,
    a.ip_address,
    b.platform
FROM smelt.ref('sql_l2_146') a
LEFT JOIN smelt.ref('py_l2_445') b ON a.user_id = b.user_id
