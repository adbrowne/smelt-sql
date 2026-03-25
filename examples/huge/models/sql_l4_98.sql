---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT amount, ip_address, profit
    FROM smelt.ref('sql_l3_203')
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
