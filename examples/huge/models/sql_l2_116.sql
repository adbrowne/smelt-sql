---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT profit, event_type, email_domain
    FROM smelt.ref('py_l1_398')
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
