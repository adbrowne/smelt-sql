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
    AVG(price) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    AVG(amount) AS agg_3,
    MIN(created_at) AS agg_4
FROM smelt.sql_l2_69
GROUP BY ip_address
