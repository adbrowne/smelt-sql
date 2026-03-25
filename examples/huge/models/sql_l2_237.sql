---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.session_id,
    a.os_name,
    b.tier
FROM smelt.ref('sql_l1_173') a
LEFT JOIN smelt.ref('sql_l1_136') b ON a.user_id = b.user_id
