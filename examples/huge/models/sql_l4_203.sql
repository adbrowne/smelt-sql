---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    ip_address,
    country
FROM smelt.sql_l3_190
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_99 WHERE country = 'US'
)

