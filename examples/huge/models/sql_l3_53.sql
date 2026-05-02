---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    browser,
    revenue
FROM smelt.models.sql_l2_18
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l2_173 WHERE score >= 50
)

