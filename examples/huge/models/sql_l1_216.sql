---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT referrer, segment, device_type
    FROM smelt.ref('orders')
    WHERE score >= 50
),
aggregated AS (
    SELECT referrer, COUNT(*) AS cnt
    FROM filtered
    GROUP BY referrer
)
SELECT
    a.referrer,
    a.cnt,
    f.segment
FROM aggregated a
INNER JOIN filtered f ON a.referrer = f.referrer
