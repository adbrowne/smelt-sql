---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.sql_l3_111
GROUP BY referrer
HAVING COUNT(*) > 10

