---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    SUM(revenue) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.sql_l2_62
GROUP BY cohort_date

