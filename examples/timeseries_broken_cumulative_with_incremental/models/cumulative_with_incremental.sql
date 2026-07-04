---
materialization: table
refresh: cumulative
batched: {}
---
SELECT device_id, COUNT(*) AS event_count
FROM some_events
GROUP BY device_id
