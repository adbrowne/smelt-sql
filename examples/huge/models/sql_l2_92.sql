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
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.models.sql_l1_76
GROUP BY referrer
HAVING COUNT(*) > 10

