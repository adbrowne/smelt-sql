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
    SELECT duration_seconds, event_type, event_date
    FROM smelt.sessions
    WHERE quantity > 0
),
aggregated AS (
    SELECT duration_seconds, COUNT(*) AS cnt
    FROM filtered
    GROUP BY duration_seconds
)
SELECT
    a.duration_seconds,
    a.cnt,
    f.event_type
FROM aggregated a
INNER JOIN filtered f ON a.duration_seconds = f.duration_seconds
