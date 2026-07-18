---
materialization: table
refresh: incremental
grain: key
unique_key: [device_id, user_id]
---
-- Cumulative (device_id, user_id) co-occurrence evidence — every signed-in
-- event contributes one observation, combined across all source partitions
-- into a single row per (device, user) pair.  The cumulative merge loop
-- derives the unique key from GROUP BY and the per-column combiner from
-- each projection's aggregator (COUNT->SUM, MIN->MIN, MAX->MAX), so each
-- daily run only re-aggregates that day's events and merges into the
-- running cumulative state.
SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_deduped
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
