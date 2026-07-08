---
materialization: table
refresh: incremental
grain: key
batched: {}
---
SELECT device_id, COUNT(*) AS event_count
FROM some_events
GROUP BY device_id
