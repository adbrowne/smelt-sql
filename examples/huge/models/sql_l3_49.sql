---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT cost, is_active, page_path
    FROM smelt.ref('sql_l2_44')
    WHERE amount > 0
)
SELECT
    b.cost,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_250') j ON b.user_id = j.user_id
GROUP BY b.cost
