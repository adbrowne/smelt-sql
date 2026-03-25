---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT is_verified, revenue, profit
    FROM smelt.ref('sql_l2_107')
    WHERE country = 'US'
),
aggregated AS (
    SELECT is_verified, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_verified
)
SELECT
    a.is_verified,
    a.cnt,
    f.revenue
FROM aggregated a
INNER JOIN filtered f ON a.is_verified = f.is_verified
