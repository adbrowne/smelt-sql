---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT ip_address, segment, os_name
    FROM smelt.models.sql_l1_47
    WHERE quantity > 0
),
aggregated AS (
    SELECT ip_address, COUNT(*) AS cnt
    FROM filtered
    GROUP BY ip_address
)
SELECT
    a.ip_address,
    a.cnt,
    f.segment
FROM aggregated a
INNER JOIN filtered f ON a.ip_address = f.ip_address

