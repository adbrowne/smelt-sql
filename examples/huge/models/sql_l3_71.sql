---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    quantity,
    AVG(duration_seconds) AS agg_0,
    AVG(price) AS agg_1,
    MAX(created_at) AS agg_2
FROM smelt.ref('py_l2_475')
GROUP BY quantity
