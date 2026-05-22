---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT cohort_date, profit, event_time
    FROM smelt.sql_l2_61
    WHERE is_active = true
)
SELECT
    b.cohort_date,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_235 j ON b.user_id = j.user_id
GROUP BY b.cohort_date
