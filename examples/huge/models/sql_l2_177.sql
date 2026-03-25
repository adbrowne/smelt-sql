---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT ip_address, is_verified, product_id
    FROM smelt.ref('py_l1_317')
    WHERE quantity > 0
)
SELECT
    b.ip_address,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_470') j ON b.user_id = j.user_id
GROUP BY b.ip_address
