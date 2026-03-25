---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    b.cost,
    c.revenue,
    c.category
FROM smelt.ref('sql_l2_189') a
INNER JOIN smelt.ref('sql_l2_18') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_189') c ON a.user_id = c.user_id
