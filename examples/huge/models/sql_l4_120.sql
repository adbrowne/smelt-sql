---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT price, ip_address, referrer
    FROM smelt.ref('sql_l3_94')
    WHERE is_active = true
),
aggregated AS (
    SELECT price, COUNT(*) AS cnt
    FROM filtered
    GROUP BY price
)
SELECT
    a.price,
    a.cnt,
    f.ip_address
FROM aggregated a
INNER JOIN filtered f ON a.price = f.price
