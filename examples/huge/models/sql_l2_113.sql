---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    user_id,
    cost,
    discount
FROM smelt.ref('sql_l1_189')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_14') WHERE status = 'active'
)
