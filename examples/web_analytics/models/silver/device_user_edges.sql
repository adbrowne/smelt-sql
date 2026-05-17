-- (device_id, user_id) co-occurrence evidence — every signed-in event
-- contributes one observation. Downstream identity algorithms (forward-only,
-- backward-fill, connected-components) consume this as the canonical edge
-- set so they all see the same evidence shape.
SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
