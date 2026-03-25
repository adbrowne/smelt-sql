---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    AVG(price) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    AVG(amount) AS agg_3,
    MIN(created_at) AS agg_4
FROM smelt.ref('py_l2_402')
GROUP BY ip_address
