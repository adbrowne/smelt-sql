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
    SELECT amount, ip_address, profit
    FROM smelt.sql_l3_27
    WHERE is_active = true
),
aggregated AS (
    SELECT amount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY amount
)
SELECT
    a.amount,
    a.cnt,
    f.ip_address
FROM aggregated a
INNER JOIN filtered f ON a.amount = f.amount

