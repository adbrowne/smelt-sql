---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    MAX(created_at) AS val_1,
    AVG(amount) AS val_2
FROM smelt.models.refunds
GROUP BY platform
HAVING COUNT(*) > 10

