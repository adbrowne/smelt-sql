---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    AVG(duration_seconds) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2
FROM smelt.ref('sql_l2_136')
GROUP BY score
