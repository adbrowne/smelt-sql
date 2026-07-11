---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Event-grain enrichment: attaches each event's `session_id` and the
-- session's attributed `utm_campaign` (first-touch, `silver/sessions`) back
-- onto the event row, alongside the event's own raw `utm_campaign` from
-- `silver/events_parsed` for comparison. Two model upstreams, both
-- maintained: `silver.events_parsed` (this model's own `event_date` clock,
-- read 1:1) and `silver.sessions` (clocked by `session_start_date`, joined
-- across the session boundary). `smelt explain silver.events_enriched`
-- shows a creation cell for each upstream, each clamped by that upstream's
-- own derived reach (`docs/specs/maintenance_plan.md` §"Upstream model
-- edges") — so a run touching one `event_date` partition only ever
-- re-touches the corresponding `event_date` partition here, never the
-- whole table.
--
-- The join carries the same 1-day session-cap Form B filter as
-- `gold/eventstream_with_identity`: a session that started on the previous
-- day can still own an event on this day (a session cannot span more than
-- `max_session_length`, `silver/sessions`' explicit cap), so declaring
-- `session_start_date` stays within 1 day of `event_date` widens the
-- `sessions` read by exactly that cap, composing with `sessions`' own
-- derived clamp rather than re-deriving it. `silver.events_parsed`'s own
-- 3-day late-arrival window (`docs/specs/datagen.md` §"Redelivery
-- (duplicate emission)") is absorbed upstream already — a late arrival
-- landing today re-touches `events_parsed`'s [D-3, D) partitions, and this
-- model's own `event_date`-clocked creation cell on that upstream
-- re-touches the same partitions here, purely through clamp composition
-- (no additional filter needed in this model's own body).
SELECT
    e.event_id,
    e.device_id,
    e.user_id,
    e.amplitude_id,
    e.event_ts,
    e.event_date,
    e.event_name,
    e.platform,
    e.url,
    e.utm_campaign AS event_utm_campaign,
    s.session_id,
    s.utm_campaign AS session_utm_campaign
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
-- Form B: the session-cap composition described above.
WHERE s.session_start_date
    BETWEEN e.event_date - INTERVAL '1 day'
        AND e.event_date + INTERVAL '1 day'
