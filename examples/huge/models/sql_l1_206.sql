---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    SUM(amount) AS agg_0,
    MAX(created_at) AS agg_1
FROM smelt.ref('reviews')
GROUP BY referrer
