---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    MAX(created_at) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.sql_l3_182
GROUP BY plan_type

