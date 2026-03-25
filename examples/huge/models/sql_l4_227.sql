---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    segment,
    event_time
FROM smelt.ref('sql_l3_81')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_152') WHERE event_type = 'purchase'
)
