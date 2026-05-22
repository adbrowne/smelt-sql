---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT country, created_at, order_id
    FROM smelt.users
    WHERE event_type = 'purchase'
)
SELECT
    b.country,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.users j ON b.user_id = j.user_id
GROUP BY b.country
