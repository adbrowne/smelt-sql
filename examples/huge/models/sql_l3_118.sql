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
    campaign_id,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.sql_l2_127
GROUP BY campaign_id
HAVING COUNT(*) > 10

