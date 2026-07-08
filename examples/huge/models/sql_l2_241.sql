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
    email_domain,
    MIN(created_at) AS agg_0,
    COUNT(*) AS agg_1,
    MAX(created_at) AS agg_2,
    SUM(quantity) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.sql_l1_81
GROUP BY email_domain
