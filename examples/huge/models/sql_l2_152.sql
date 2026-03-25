---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    a.order_id,
    b.campaign_id
FROM smelt.ref('sql_l1_105') a
INNER JOIN smelt.ref('py_l1_450') b ON a.user_id = b.user_id
