---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT created_at, revenue, user_id
    FROM smelt.ref('py_l1_432')
    WHERE quantity > 0
),
aggregated AS (
    SELECT created_at, COUNT(*) AS cnt
    FROM filtered
    GROUP BY created_at
)
SELECT
    a.created_at,
    a.cnt,
    f.revenue
FROM aggregated a
INNER JOIN filtered f ON a.created_at = f.created_at
