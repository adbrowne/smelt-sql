---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    campaign_id,
    amount
FROM smelt.sql_l1_211
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_147 WHERE created_at >= '2024-01-01'
)

