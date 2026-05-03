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
    SUM(quantity) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.sql_l2_166
GROUP BY campaign_id
HAVING COUNT(*) > 10

