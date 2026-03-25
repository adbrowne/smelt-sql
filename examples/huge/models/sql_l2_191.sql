---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    AVG(price) AS agg_0,
    MAX(created_at) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3,
    SUM(revenue) AS agg_4
FROM smelt.ref('py_l1_407')
GROUP BY campaign_id
