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
    SELECT country, is_verified, browser
    FROM smelt.errors
    WHERE platform = 'web'
),
aggregated AS (
    SELECT country, COUNT(*) AS cnt
    FROM filtered
    GROUP BY country
)
SELECT
    a.country,
    a.cnt,
    f.is_verified
FROM aggregated a
INNER JOIN filtered f ON a.country = f.country
