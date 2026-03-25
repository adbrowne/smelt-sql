---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT event_time, category, channel
    FROM smelt.ref('invoices')
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT event_time, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_time
)
SELECT
    a.event_time,
    a.cnt,
    f.category
FROM aggregated a
INNER JOIN filtered f ON a.event_time = f.event_time
