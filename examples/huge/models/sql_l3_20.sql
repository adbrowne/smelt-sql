---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    quantity,
    country
FROM smelt.ref('sql_l2_121')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_219') WHERE event_type = 'purchase'
)
