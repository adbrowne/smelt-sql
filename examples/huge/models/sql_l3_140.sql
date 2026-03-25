---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT os_name, status, platform
    FROM smelt.ref('py_l2_485')
    WHERE score >= 50
),
aggregated AS (
    SELECT os_name, COUNT(*) AS cnt
    FROM filtered
    GROUP BY os_name
)
SELECT
    a.os_name,
    a.cnt,
    f.status
FROM aggregated a
INNER JOIN filtered f ON a.os_name = f.os_name
