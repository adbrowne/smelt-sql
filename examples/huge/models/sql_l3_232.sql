---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    platform,
    campaign_id
FROM smelt.sql_l2_166
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_206 WHERE category IS NOT NULL
)

