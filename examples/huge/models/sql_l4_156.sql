---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    status,
    created_at
FROM smelt.ref('sql_l3_23')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_141') WHERE quantity > 0
)
