---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cost,
    b.country,
    c.created_at,
    c.event_date
FROM smelt.ref('sql_l3_218') a
INNER JOIN smelt.ref('sql_l3_218') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_218') c ON a.user_id = c.user_id
