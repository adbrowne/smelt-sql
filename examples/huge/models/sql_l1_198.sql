---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT os_name, region, session_id
    FROM smelt.ref('clicks')
    WHERE quantity > 0
),
aggregated AS (
    SELECT os_name, COUNT(*) AS cnt
    FROM filtered
    GROUP BY os_name
)
SELECT
    a.os_name,
    a.cnt,
    f.region
FROM aggregated a
INNER JOIN filtered f ON a.os_name = f.os_name
