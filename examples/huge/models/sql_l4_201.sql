---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    rating,
    ROW_NUMBER() OVER (PARTITION BY campaign_id ORDER BY created_at) AS win_val
FROM smelt.sql_l3_112

