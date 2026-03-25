---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    SUM(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    MIN(created_at) AS agg_2,
    COUNT(*) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.ref('orders')
GROUP BY referrer
