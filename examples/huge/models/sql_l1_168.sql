---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT platform, region, status
    FROM smelt.notifications
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT platform, COUNT(*) AS cnt
    FROM filtered
    GROUP BY platform
)
SELECT
    a.platform,
    a.cnt,
    f.region
FROM aggregated a
INNER JOIN filtered f ON a.platform = f.platform

