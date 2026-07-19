---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Per-event wide table that joins every silver/events_deduped row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved amplitude_id. The wide shape carries the no-merging baseline
-- (silver's amplitude_id) plus three refinements (forward_only, backward_fill,
-- connected_components); additional algorithms can be added as LEFT JOIN +
-- column projections without restructuring the row.
--
-- Every *_amplitude_id column is non-null because the identity-model NULLs
-- (sessions / devices missing from the upstream tables) are COALESCEd into
-- the device-prefix amplitude_id ('d:' || device_id). This matches Amplitude's
-- behaviour — every event has an identity, methods differ only in how
-- aggressively they merge that identity space.
--
-- Columns:
--   event_id              — opaque event identifier from raw ingestion
--   device_id             — the device that generated the event
--   event_user_id         — raw user_id observation on the event (nullable)
--   amplitude_id          — silver's no-merging baseline: 'u:user_id' when
--                           signed in, 'd:device_id' otherwise. Always non-null.
--   event_ts              — timestamp of the event
--   event_date            — calendar date of the event (partition key in raw)
--   event_name            — decoded event name from the JSON payload
--   platform              — decoded platform from the JSON payload
--   url                   — decoded url from the JSON payload
--   session_id            — the session this event belongs to
--   forward_only_amplitude_id  — within-session refinement: the latest signed-in
--                                user wins for the whole session; sessions with
--                                zero signed-in events fall back to 'd:device_id'.
--   backward_fill_amplitude_id — per-device refinement: the most-frequent
--                                signed-in user (first_seen / user_id tiebreaks)
--                                back-tags every event on the device. Devices
--                                with zero signed-in events fall back to
--                                'd:device_id'.
--   connected_components_amplitude_id — cross-device refinement: the smallest
--                                user_id in the bipartite (device, user)
--                                connected component. Devices not in the graph
--                                fall back to 'd:device_id'.
--   connected_components_cluster_id   — cluster label from the union-find;
--                                numerically equal to
--                                connected_components_amplitude_id in v1 but
--                                surfaced separately so a future
--                                probabilistic-stitching alternative could
--                                decouple them without reshuffling the
--                                eventstream.
SELECT
    e.event_id,
    e.device_id,
    e.user_id AS event_user_id,
    e.amplitude_id,
    e.event_ts,
    e.event_date,
    e.event_name,
    e.platform,
    e.url,
    s.session_id,
    COALESCE(f.forward_only_amplitude_id,        'd:' || CAST(e.device_id AS VARCHAR)) AS forward_only_amplitude_id,
    COALESCE(b.backward_fill_amplitude_id,       'd:' || CAST(e.device_id AS VARCHAR)) AS backward_fill_amplitude_id,
    COALESCE(c.connected_components_amplitude_id, 'd:' || CAST(e.device_id AS VARCHAR)) AS connected_components_amplitude_id,
    COALESCE(c.connected_components_cluster_id,   'd:' || CAST(e.device_id AS VARCHAR)) AS connected_components_cluster_id
FROM smelt.silver.events_deduped e
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
-- Form B: a session that started on the previous day can own an event on this
-- day across midnight. Declaring that session_start_date stays within 1 day of
-- event_date widens the sessions read to the previous partition, so the day-D
-- event still finds its D-1-started session. Kept as a WHERE filter rather than
-- a JOIN condition so the incremental safety classifier reads it cleanly.
WHERE s.session_start_date
    BETWEEN e.event_date - INTERVAL '1 day'
        AND e.event_date + INTERVAL '1 day'
-- Form B: this model's own `event_date` and `silver.events_deduped`'s
-- `first_seen_date` are the same value by construction (both are
-- `MIN(event_date)` per `event_id` upstream) — a true 1:1, zero-skew read.
-- The planner's cross-axis Form B derivation only registers a *nonzero*
-- margin (a same-name, same-axis zero margin is derived separately, and
-- this model's own declared `partition_column` stays `event_date` — it is
-- not renamed to `first_seen_date` because `marts.daily_active_users_by_method`
-- and its own tests already read this model under the `event_date` name).
-- This filter restates the tautology as an explicit, conservative 1-day
-- bound so `silver.events_deduped`'s read stays partition-pruned rather
-- than falling back to an unbounded scan.
  AND e.first_seen_date
      BETWEEN e.event_date - INTERVAL '1 day'
          AND e.event_date + INTERVAL '1 day'
