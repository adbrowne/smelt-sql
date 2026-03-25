---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    AVG(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(amount) AS agg_2,
    COUNT(*) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.ref('py_l2_429')
GROUP BY ip_address
