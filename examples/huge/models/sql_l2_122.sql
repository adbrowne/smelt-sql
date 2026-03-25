---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT rating, plan_type, event_time
    FROM smelt.ref('py_l1_265')
    WHERE score >= 50
),
aggregated AS (
    SELECT rating, COUNT(*) AS cnt
    FROM filtered
    GROUP BY rating
)
SELECT
    a.rating,
    a.cnt,
    f.plan_type
FROM aggregated a
INNER JOIN filtered f ON a.rating = f.rating
