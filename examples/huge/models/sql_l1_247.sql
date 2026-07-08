---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT event_date, duration_seconds, is_verified
    FROM smelt.campaigns
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT event_date, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_date
)
SELECT
    a.event_date,
    a.cnt,
    f.duration_seconds
FROM aggregated a
INNER JOIN filtered f ON a.event_date = f.event_date
