---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1
FROM smelt.shipments
GROUP BY os_name

