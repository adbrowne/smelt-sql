---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1,
    SUM(quantity) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3
FROM smelt.ref('sql_l1_39')
GROUP BY country
