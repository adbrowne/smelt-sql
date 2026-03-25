---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    SUM(revenue) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('page_views')
GROUP BY ip_address
HAVING COUNT(*) > 10
