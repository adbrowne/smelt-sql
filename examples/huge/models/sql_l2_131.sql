---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    cost,
    campaign_id
FROM smelt.sql_l1_27
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_65 WHERE country = 'US'
)

