---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1
FROM smelt.sql_l3_96
GROUP BY channel
