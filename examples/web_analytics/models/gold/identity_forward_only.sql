-- Per-session identity resolution using the forward-only algorithm.
--
-- Produces one row per session with (session_id, forward_only_user_id).
-- forward_only_user_id is the user_id observed at the event with the latest
-- event_ts among non-null observations within the session window. NULL when
-- the session contains zero signed-in events.
--
-- This is the simplest identity algorithm: no cross-session propagation, no
-- per-device canonical-user election, no edge clustering. Each session is
-- resolved independently from its own events.
SELECT
    s.session_id,
    arg_max(e.user_id, e.event_ts) FILTER (WHERE e.user_id IS NOT NULL) AS forward_only_user_id
FROM smelt.silver.sessions s
JOIN smelt.silver.events_parsed e
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
GROUP BY s.session_id
