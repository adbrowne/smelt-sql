---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT cost, order_id, browser
    FROM smelt.ref('sql_l2_222')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.cost,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_127') j ON b.user_id = j.user_id
GROUP BY b.cost
