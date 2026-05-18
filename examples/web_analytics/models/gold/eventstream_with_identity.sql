-- Per-event wide table that joins every silver/events_parsed row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved column. Carries three identity algorithms (forward_only,
-- backward_fill, connected_components); the wide shape is fixed so additional
-- algorithms can be added as LEFT JOIN + column projections without
-- restructuring the row.
--
-- Columns:
--   event_id              — opaque event identifier from raw ingestion
--   device_id             — the device that generated the event
--   event_user_id         — raw user_id observation on the event (nullable)
--   event_ts              — timestamp of the event
--   event_date            — calendar date of the event (partition key in raw)
--   event_name            — decoded event name from the JSON payload
--   platform              — decoded platform from the JSON payload
--   url                   — decoded url from the JSON payload
--   session_id            — the session this event belongs to
--   forward_only_user_id  — resolved identity via the within-session algorithm
--                           (NULL for sessions with zero signed-in events)
--   backward_fill_user_id — resolved identity via the per-device canonical-user
--                           election (NULL for devices that never had a
--                           signed-in event); see gold/identity_backward_fill
--   connected_components_user_id  — resolved identity via the cross-device
--                           bipartite-graph union-find (NULL for devices that
--                           never had a signed-in event); see
--                           gold/identity_connected_components
--   connected_components_cluster_id — cluster label from the connected-components
--                           union-find (NULL on the same condition as the
--                           user_id column). Numerically equal to
--                           connected_components_user_id in the v1 algorithm
--                           (both are the smallest user_id in the cluster);
--                           surfaced separately so a future probabilistic-
--                           stitching alternative could decouple them without
--                           reshuffling the eventstream.
SELECT
    e.event_id,
    e.device_id,
    e.user_id AS event_user_id,
    e.event_ts,
    e.event_date,
    e.event_name,
    e.platform,
    e.url,
    s.session_id,
    f.forward_only_user_id,
    b.backward_fill_user_id,
    c.connected_components_user_id,
    c.connected_components_cluster_id
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
LEFT JOIN smelt.gold.identity_forward_only f
    ON s.session_id = f.session_id
LEFT JOIN smelt.gold.identity_backward_fill b
    ON e.device_id = b.device_id
LEFT JOIN smelt.gold.identity_connected_components c
    ON e.device_id = c.device_id
