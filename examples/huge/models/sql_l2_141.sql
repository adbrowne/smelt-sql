---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT event_type, os_name, email_domain
    FROM smelt.models.sql_l1_75
    WHERE is_active = true
),
aggregated AS (
    SELECT event_type, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_type
)
SELECT
    a.event_type,
    a.cnt,
    f.os_name
FROM aggregated a
INNER JOIN filtered f ON a.event_type = f.event_type

