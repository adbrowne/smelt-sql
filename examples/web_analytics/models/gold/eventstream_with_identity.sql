-- Per-event wide table that joins every silver/events_parsed row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved column. Carries one identity column today (forward_only); the wide
-- shape is fixed so additional algorithms can be added as LEFT JOIN + one
-- column projection without restructuring the row.
--
-- Columns:
--   event_id           — opaque event identifier from raw ingestion
--   device_id          — the device that generated the event
--   event_user_id      — raw user_id observation on the event (nullable)
--   event_ts           — timestamp of the event
--   event_date         — calendar date of the event (partition key in raw)
--   event_name         — decoded event name from the JSON payload
--   platform           — decoded platform from the JSON payload
--   url                — decoded url from the JSON payload
--   session_id         — the session this event belongs to
--   forward_only_user_id — resolved identity via the forward-only algorithm
--                          (NULL for sessions with zero signed-in events)
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
    f.forward_only_user_id
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
LEFT JOIN smelt.gold.identity_forward_only f
    ON s.session_id = f.session_id
