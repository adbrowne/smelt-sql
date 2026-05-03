---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    cohort_date,
    order_id
FROM smelt.sql_l2_3
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_97 WHERE amount > 0
)

