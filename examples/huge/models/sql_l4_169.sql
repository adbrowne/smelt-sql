---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    SUM(quantity) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.ref('sql_l3_196')
GROUP BY rating
