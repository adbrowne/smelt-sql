-- Cumulative (device_id, user_id) co-occurrence evidence across all dates,
-- rolled up from the per-day silver/device_user_edges table.  This view exists
-- so the two global identity algorithms (backward_fill, connected_components)
-- can read a single canonical edge set without each daily run re-aggregating
-- the full silver/events_parsed history.
--
-- Output column names exactly match the original pre-incremental
-- silver/device_user_edges shape so downstream models keep their existing
-- column references unchanged.
SELECT
    device_id,
    user_id,
    SUM(daily_event_count) AS event_count,
    MIN(daily_first_seen) AS first_seen,
    MAX(daily_last_seen) AS last_seen
FROM smelt.silver.device_user_edges
GROUP BY device_id, user_id
