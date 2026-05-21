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
    SELECT status, revenue, browser
    FROM smelt.events
    WHERE event_type = 'purchase'
)
SELECT
    b.status,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.events j ON b.user_id = j.user_id
GROUP BY b.status

