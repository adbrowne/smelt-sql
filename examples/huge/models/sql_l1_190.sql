---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT country, created_at, order_id
    FROM smelt.models.users
    WHERE event_type = 'purchase'
)
SELECT
    b.country,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.models.users j ON b.user_id = j.user_id
GROUP BY b.country

