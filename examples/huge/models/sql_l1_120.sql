---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT channel, rating, os_name
    FROM smelt.subscriptions
    WHERE score >= 50
),
aggregated AS (
    SELECT channel, COUNT(*) AS cnt
    FROM filtered
    GROUP BY channel
)
SELECT
    a.channel,
    a.cnt,
    f.rating
FROM aggregated a
INNER JOIN filtered f ON a.channel = f.channel
