---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT cohort_date, is_verified, event_type
    FROM smelt.ref('py_l3_469')
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT cohort_date, COUNT(*) AS cnt
    FROM filtered
    GROUP BY cohort_date
)
SELECT
    a.cohort_date,
    a.cnt,
    f.is_verified
FROM aggregated a
INNER JOIN filtered f ON a.cohort_date = f.cohort_date
