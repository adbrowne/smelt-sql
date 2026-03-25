---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT ip_address, is_verified, product_id
    FROM smelt.ref('sql_l1_124')
    WHERE quantity > 0
)
SELECT
    b.ip_address,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_0') j ON b.user_id = j.user_id
GROUP BY b.ip_address
