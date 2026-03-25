---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.ref('sql_l3_208')
GROUP BY order_id
