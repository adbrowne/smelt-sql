---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT is_active, discount, duration_seconds
    FROM smelt.ref('py_l1_338')
    WHERE country = 'US'
),
aggregated AS (
    SELECT is_active, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_active
)
SELECT
    a.is_active,
    a.cnt,
    f.discount
FROM aggregated a
INNER JOIN filtered f ON a.is_active = f.is_active
