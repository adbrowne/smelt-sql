---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT profit, cohort_date, transaction_id
    FROM smelt.sql_l1_170
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT profit, COUNT(*) AS cnt
    FROM filtered
    GROUP BY profit
)
SELECT
    a.profit,
    a.cnt,
    f.cohort_date
FROM aggregated a
INNER JOIN filtered f ON a.profit = f.profit

