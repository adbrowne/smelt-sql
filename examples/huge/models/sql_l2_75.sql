---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT is_verified, created_at, is_active
    FROM smelt.ref('py_l1_314')
    WHERE platform = 'web'
),
aggregated AS (
    SELECT is_verified, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_verified
)
SELECT
    a.is_verified,
    a.cnt,
    f.created_at
FROM aggregated a
INNER JOIN filtered f ON a.is_verified = f.is_verified
