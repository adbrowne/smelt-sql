---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Tutorial stage 5 (see docs-site examples/web-analytics): event-grain
-- enrichment joining each event's session identity back onto the event row,
-- alongside the event's own raw utm_campaign for comparison. Same as the
-- full example's silver.events_enriched minus the root-anchored
-- sessions_chained join (introduced there as the alternative design).
-- Two model upstreams, both maintained: silver.events_parsed (this model's
-- own event_date clock, read 1:1) and silver.sessions (clocked by
-- session_start_date, joined across the session boundary). The WHERE filter
-- declares how far a session's start date can sit from an event's own date
-- (one day either way, the session table's own cap), so smelt widens the
-- sessions read by exactly that much and no more.
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
WHERE s.session_start_date
    BETWEEN e.event_date - INTERVAL '1 day'
        AND e.event_date + INTERVAL '1 day'
