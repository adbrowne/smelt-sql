---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT cohort_date, is_verified, tier
    FROM smelt.sql_l2_157
    WHERE country = 'US'
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

