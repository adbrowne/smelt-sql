---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    discount,
    browser
FROM smelt.ref('sql_l2_237')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_67') WHERE quantity > 0
)
