---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---

SELECT event_date, 'orders' AS source_type, COUNT(*) AS n
FROM orders
GROUP BY event_date
UNION ALL
SELECT event_date, 'returns' AS source_type, COUNT(*) AS n
FROM returns
GROUP BY event_date
