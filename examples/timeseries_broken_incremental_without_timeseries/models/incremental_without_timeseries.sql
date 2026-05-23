---
materialization: table
incremental:
  enabled: true
---
SELECT event_date, user_id, COUNT(*) AS event_count
FROM events
GROUP BY 1, 2
