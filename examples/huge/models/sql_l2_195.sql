---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    SUM(revenue) AS agg_0,
    MAX(created_at) AS agg_1,
    AVG(price) AS agg_2
FROM smelt.ref('py_l1_347')
GROUP BY updated_at
