---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    COUNT(*) AS agg_0,
    SUM(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.ref('py_l3_443')
GROUP BY event_type
