---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT profit, email_domain, device_type
    FROM smelt.ref('sql_l2_84')
    WHERE quantity > 0
)
SELECT
    b.profit,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_208') j ON b.user_id = j.user_id
GROUP BY b.profit
