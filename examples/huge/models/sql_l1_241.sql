---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.invoices
GROUP BY event_date
HAVING COUNT(*) > 10
