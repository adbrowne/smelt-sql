---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    updated_at,
    SUM(revenue) AS agg_0,
    MAX(created_at) AS agg_1,
    AVG(price) AS agg_2
FROM smelt.ref('sql_l1_93')
GROUP BY updated_at
