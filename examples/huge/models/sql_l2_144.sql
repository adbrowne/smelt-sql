---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.transaction_id,
    a.os_name,
    b.referrer
FROM smelt.ref('sql_l1_90') a
LEFT JOIN smelt.ref('sql_l1_90') b ON a.user_id = b.user_id
