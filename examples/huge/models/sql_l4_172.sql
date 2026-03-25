---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    AVG(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    COUNT(*) AS agg_2,
    MAX(created_at) AS agg_3
FROM smelt.ref('sql_l3_136')
GROUP BY profit
