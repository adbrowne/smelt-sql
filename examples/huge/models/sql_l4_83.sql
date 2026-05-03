---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    campaign_id,
    channel,
    browser
FROM smelt.sql_l3_8
WHERE category IS NOT NULL

