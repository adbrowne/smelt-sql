---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    SUM(quantity) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.sql_l3_43
GROUP BY ip_address
HAVING COUNT(*) > 10
