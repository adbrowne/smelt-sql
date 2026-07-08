---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT profit, event_type, email_domain
    FROM smelt.sql_l1_125
    WHERE score >= 50
),
aggregated AS (
    SELECT profit, COUNT(*) AS cnt
    FROM filtered
    GROUP BY profit
)
SELECT
    a.profit,
    a.cnt,
    f.event_type
FROM aggregated a
INNER JOIN filtered f ON a.profit = f.profit
