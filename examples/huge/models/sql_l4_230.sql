---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT country, profit, price
    FROM smelt.ref('sql_l3_21')
    WHERE is_active = true
)
SELECT
    b.country,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_94') j ON b.user_id = j.user_id
GROUP BY b.country
