---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.duration_seconds,
    a.referrer,
    b.page_path
FROM smelt.ref('clicks') a
LEFT JOIN smelt.ref('clicks') b ON a.user_id = b.user_id
