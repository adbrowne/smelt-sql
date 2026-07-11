---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
# The redelivery dedup window partitions by event_id, not event_date — the
# analyzer cannot statically prove it is partition-aligned. It is safe in
# practice: a redelivered duplicate is always written into the *same*
# event_date partition as its original (`docs/specs/datagen.md`
# §"Redelivery"), so the window never needs to see across a partition
# boundary to resolve one event_id's duplicates.
batched:
  safety_overrides:
    allow_window_functions: true
---
-- Compose event_ts from the datagen-provided occurrence clock, project the
-- JSON payload fields as typed columns via parse_event_payload, and
-- synthesise the amplitude_id — the Amplitude-style never-NULL identifier
-- that prefers the signed-in user_id and falls back to the device_id. The
-- 'u:' / 'd:' prefix keeps the user-id and device-id namespaces disjoint so
-- cross-device collisions are impossible. amplitude_id is the no-merging
-- baseline that the three gold-layer identity algorithms refine into more
-- aggressive partitions.
--
-- Two upstream-hygiene concerns land here, ahead of every identity
-- refinement:
--
-- - **Redelivery.** The bronze feed is at-least-once: a small fraction of
--   events arrive twice, byte-identical except for `arrival_time` (the
--   redelivered copy's arrival is later). `QUALIFY ROW_NUMBER() OVER
--   (PARTITION BY event_id ORDER BY arrival_time) = 1` keeps the
--   earliest-arriving copy per `event_id` and drops the redelivered
--   duplicate.
-- - **Lateness.** An event's ingestion (`arrival_time`) can trail its
--   occurrence (`event_time`) by up to 3 days. The Form B filter below
--   declares the accepted window: the planner reads `event_date BETWEEN
--   CAST(arrival_time AS DATE) - INTERVAL '3 days' AND CAST(arrival_time AS
--   DATE)` as a genuine 3-day lookback on the `bronze.raw_events` source
--   (derived, not declared — visible via `smelt explain
--   silver.events_parsed`), so a run touching day D also re-touches the
--   [D-3, D) partitions, re-absorbing a late arrival that had not yet landed
--   when those partitions were first written.
SELECT
    event_id,
    device_id,
    user_id,
    CASE WHEN user_id IS NOT NULL
         THEN 'u:' || CAST(user_id AS VARCHAR)
         ELSE 'd:' || CAST(device_id AS VARCHAR)
    END AS amplitude_id,
    CAST(event_time AS TIMESTAMP) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    utm_campaign,
    smelt.functions.parse_event_payload(payload).*
FROM smelt.bronze.raw_events
WHERE event_date
    BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
        AND CAST(arrival_time AS DATE)
QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
