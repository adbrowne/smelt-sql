---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT updated_at, duration_seconds, platform
    FROM smelt.ref('py_l3_281')
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
