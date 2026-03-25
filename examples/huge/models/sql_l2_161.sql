---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT quantity, os_name, price
    FROM smelt.ref('py_l1_290')
    WHERE status = 'active'
),
aggregated AS (
    SELECT quantity, COUNT(*) AS cnt
    FROM filtered
    GROUP BY quantity
)
SELECT
    a.quantity,
    a.cnt,
    f.os_name
FROM aggregated a
INNER JOIN filtered f ON a.quantity = f.quantity
