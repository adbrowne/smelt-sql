---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_date,
    b.event_time,
    c.score,
    c.channel
FROM smelt.ref('py_l3_431') a
INNER JOIN smelt.ref('sql_l3_213') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_498') c ON a.user_id = c.user_id
