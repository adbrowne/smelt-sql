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
    page_path,
    referrer,
    ip_address
FROM smelt.sql_l1_127
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_63 WHERE score >= 50
)
