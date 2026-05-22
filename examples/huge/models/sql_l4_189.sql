---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT amount, session_id, region
    FROM smelt.sql_l3_238
    WHERE score >= 50
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.session_id
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount
