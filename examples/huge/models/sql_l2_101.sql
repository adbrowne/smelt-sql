---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    COUNT(DISTINCT user_id) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2,
    AVG(duration_seconds) AS agg_3
FROM smelt.ref('sql_l1_88')
GROUP BY discount
