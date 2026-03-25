---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT is_active, platform, duration_seconds
    FROM smelt.ref('sql_l1_10')
    WHERE platform = 'web'
),
aggregated AS (
    SELECT is_active, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_active
)
SELECT
    a.is_active,
    a.cnt,
    f.platform
FROM aggregated a
INNER JOIN filtered f ON a.is_active = f.is_active
