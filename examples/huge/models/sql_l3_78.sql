---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT amount, cohort_date, plan_type
    FROM smelt.sql_l2_175
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.cohort_date
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount

