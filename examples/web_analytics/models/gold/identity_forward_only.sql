---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- Per-session identity resolution using the forward-only algorithm.
--
-- Produces one row per session with (session_id, session_start_date,
-- forward_only_amplitude_id).  forward_only_amplitude_id is 'u:' || the
-- user_id observed at the event with the latest event_ts among non-null
-- observations within the session window.  NULL when the session contains
-- zero signed-in events; the eventstream layer COALESCEs that NULL into
-- the device-prefix amplitude_id ('d:' || device_id) downstream so every
-- event ends up with a non-null resolved amplitude_id.
--
-- This is the simplest identity algorithm: no cross-session propagation,
-- no per-device canonical-user election, no edge clustering.  Each session
-- is resolved independently from its own events.
--
-- session_start_date is the incremental partition_column; it appears in
-- both the SELECT list and the GROUP BY.  The 1-day driver lookback (run
-- with --event-time-start D-1 --event-time-end D+1) catches sessions whose
-- latest signed-in event arrives the day after session start.
SELECT
    s.session_id,
    s.session_start_date,
    'u:' || CAST(arg_max(e.user_id, e.event_ts) FILTER (WHERE e.user_id IS NOT NULL) AS VARCHAR) AS forward_only_amplitude_id
FROM smelt.silver.sessions s
JOIN smelt.silver.events_parsed e
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
GROUP BY s.session_id, s.session_start_date
