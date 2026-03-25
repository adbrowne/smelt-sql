---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT ip_address, user_id, referrer
    FROM smelt.ref('sql_l2_38')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT ip_address, COUNT(*) AS cnt
    FROM filtered
    GROUP BY ip_address
)
SELECT
    a.ip_address,
    a.cnt,
    f.user_id
FROM aggregated a
INNER JOIN filtered f ON a.ip_address = f.ip_address
