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
    SELECT event_date, revenue, rating
    FROM smelt.products
    WHERE category IS NOT NULL
)
SELECT
    b.event_date,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.products j ON b.user_id = j.user_id
GROUP BY b.event_date

