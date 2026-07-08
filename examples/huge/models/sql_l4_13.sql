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
    product_id,
    COUNT(*) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.sql_l3_21
GROUP BY product_id
HAVING COUNT(*) > 10
