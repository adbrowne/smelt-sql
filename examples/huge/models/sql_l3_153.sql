---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT channel, is_verified, category
    FROM smelt.sql_l2_143
    WHERE status = 'active'
),
aggregated AS (
    SELECT channel, COUNT(*) AS cnt
    FROM filtered
    GROUP BY channel
)
SELECT
    a.channel,
    a.cnt,
    f.is_verified
FROM aggregated a
INNER JOIN filtered f ON a.channel = f.channel
