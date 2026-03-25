---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT created_at, country, session_id
    FROM smelt.ref('py_l2_489')
    WHERE amount > 0
),
aggregated AS (
    SELECT created_at, COUNT(*) AS cnt
    FROM filtered
    GROUP BY created_at
)
SELECT
    a.created_at,
    a.cnt,
    f.country
FROM aggregated a
INNER JOIN filtered f ON a.created_at = f.created_at
