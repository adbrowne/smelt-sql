---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    SUM(quantity) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.ref('py_l3_475')
GROUP BY session_id
