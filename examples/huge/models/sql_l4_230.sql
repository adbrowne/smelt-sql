---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT country, profit, price
    FROM smelt.ref('sql_l3_238')
    WHERE is_active = true
)
SELECT
    b.country,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_173') j ON b.user_id = j.user_id
GROUP BY b.country
