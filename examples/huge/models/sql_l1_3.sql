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
    SELECT country, page_path, quantity
    FROM smelt.notifications
    WHERE platform = 'web'
)
SELECT
    b.country,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.notifications j ON b.user_id = j.user_id
GROUP BY b.country
