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
    ip_address,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.notifications
GROUP BY ip_address
HAVING COUNT(*) > 10

