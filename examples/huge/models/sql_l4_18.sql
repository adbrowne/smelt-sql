---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    AVG(price) AS agg_0,
    SUM(amount) AS agg_1,
    AVG(amount) AS agg_2,
    COUNT(*) AS agg_3,
    SUM(quantity) AS agg_4
FROM smelt.ref('sql_l3_139')
GROUP BY event_type
