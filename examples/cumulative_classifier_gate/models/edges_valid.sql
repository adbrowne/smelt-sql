---
materialization: table
refresh: keyed
---
SELECT
    device_id,
    user_id,
    COUNT(*)    AS event_count,
    MIN(amount) AS min_amount,
    MAX(amount) AS max_amount
FROM smelt.events_ts
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
