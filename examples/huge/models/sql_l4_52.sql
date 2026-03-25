---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT updated_at, duration_seconds, platform
    FROM smelt.ref('sql_l3_161')
    WHERE platform = 'web'
),
aggregated AS (
    SELECT updated_at, COUNT(*) AS cnt
    FROM filtered
    GROUP BY updated_at
)
SELECT
    a.updated_at,
    a.cnt,
    f.duration_seconds
FROM aggregated a
INNER JOIN filtered f ON a.updated_at = f.updated_at
