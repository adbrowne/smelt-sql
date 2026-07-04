---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    COUNT(*) AS agg_0,
    SUM(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.sql_l3_181
GROUP BY event_type
