---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT profit, country, category
    FROM smelt.ref('sql_l2_71')
    WHERE country = 'US'
)
SELECT
    b.profit,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_8') j ON b.user_id = j.user_id
GROUP BY b.profit
