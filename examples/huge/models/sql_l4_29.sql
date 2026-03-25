---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.referrer,
    a.channel,
    b.status
FROM smelt.ref('sql_l3_195') a
INNER JOIN smelt.ref('sql_l3_67') b ON a.user_id = b.user_id
