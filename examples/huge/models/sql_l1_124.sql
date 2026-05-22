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
    SELECT updated_at, event_date, duration_seconds
    FROM smelt.notifications
    WHERE event_type = 'purchase'
)
SELECT
    b.updated_at,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.notifications j ON b.user_id = j.user_id
GROUP BY b.updated_at
