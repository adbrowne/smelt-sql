---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT cohort_date, transaction_id, country
    FROM smelt.ref('sql_l3_247')
    WHERE quantity > 0
),
aggregated AS (
    SELECT cohort_date, COUNT(*) AS cnt
    FROM filtered
    GROUP BY cohort_date
)
SELECT
    a.cohort_date,
    a.cnt,
    f.transaction_id
FROM aggregated a
INNER JOIN filtered f ON a.cohort_date = f.cohort_date
