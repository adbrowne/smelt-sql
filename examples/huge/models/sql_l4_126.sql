---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT order_id, ip_address, profit
    FROM smelt.ref('sql_l3_161')
    WHERE amount > 0
),
aggregated AS (
    SELECT order_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY order_id
)
SELECT
    a.order_id,
    a.cnt,
    f.ip_address
FROM aggregated a
INNER JOIN filtered f ON a.order_id = f.order_id
