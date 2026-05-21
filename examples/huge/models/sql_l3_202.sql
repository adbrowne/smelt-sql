---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    discount,
    browser
FROM smelt.sql_l2_237
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_67 WHERE quantity > 0
)

