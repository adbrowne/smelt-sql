---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    a.order_id,
    b.campaign_id
FROM smelt.ref('sql_l1_13') a
INNER JOIN smelt.ref('sql_l1_106') b ON a.user_id = b.user_id
