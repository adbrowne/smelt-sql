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
    revenue,
    page_path
FROM smelt.sql_l3_157
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_14 WHERE score >= 50
)

