---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Event-grain enrichment: attaches each event's session identity — under
-- BOTH upstream session tables' cut rules — back onto the event row,
-- alongside the event's own raw `utm_campaign` from `silver/events_deduped`
-- for comparison. Three model upstreams, all maintained:
-- `silver.events_deduped` (this model's own `event_date` clock, read 1:1 —
-- the composed keyed+timeseries dedupe stage, `docs/specs/
-- incremental_shapes.md` §"Key temporal locality (the time-partitioned
-- output)"; its own settle bound and clock propagate through exactly like a
-- declared source), `silver.sessions` (clock-anchored cut, clocked by
-- `session_start_date`, joined across the session boundary), and
-- `silver.sessions_chained` (root-anchored cut, self-referential, same
-- partition-column shape). The two session upstreams differ only in
-- *where the cap's phase comes from* (the clock vs. the session's own root
-- — `docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"Design"); `session_id`/`session_utm_campaign` (from `silver.sessions`)
-- stay primary and are what the gold identity models consume — the
-- root-anchored pair is additive, for direct per-event comparison.
-- `smelt explain silver.events_enriched` shows a creation cell for each of
-- the three upstreams, each clamped by that upstream's own derived reach
-- (`docs/specs/incremental_models.md` §"Upstream model edges") — so a run
-- touching one `event_date` partition only ever re-touches the
-- corresponding `event_date` partition here, never the whole table.
--
-- Each session join carries a bounded Form B filter mirroring the same
-- pattern as `gold/eventstream_with_identity`: a session can still own an
-- event on a later day than it rooted on (a session cannot outlive its own
-- table's explicit cap), so declaring `session_start_date` stays within the
-- cap of `event_date` widens that table's read by exactly that cap,
-- composing with the upstream's own derived clamp rather than re-deriving
-- it.
--   * `silver.sessions`: 1-day cap (`max_session_length` — every session
--     spans at most two calendar days by the clock-anchored closed form).
--   * `silver.sessions_chained`: 2-day cap, wider than `sessions`' own
--     because the root-anchored cut's cap is asserted as a raw duration
--     (`< INTERVAL '2 days'` in `sessions_chained.sql`'s own `HAVING`)
--     rather than a clock-aligned deadline — the 2-day bound is the
--     conservative reach that covers it.
-- `silver.events_deduped`'s own 3-day late-arrival window (both the
-- lateness acceptance filter and the recurrence-bounded dedupe —
-- `docs/specs/datagen.md` §"Redelivery (duplicate emission)") is absorbed
-- upstream already — a late arrival landing today re-touches
-- `events_deduped`'s [D-3, D) partitions, and this model's own
-- `event_date`-clocked creation cell on that upstream re-touches the same
-- partitions here, purely through clamp composition (no additional filter
-- needed in this model's own body).
--
-- This model's own declared `partition_column` stays `event_date` (not
-- renamed to match `events_deduped`'s `first_seen_date`) because
-- `events_enriched`'s own output is read under that name elsewhere
-- (`examples/web_analytics/tests/enrichment_dual_session_invariants.test.sql`,
-- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`). `event_date`
-- and `first_seen_date` are the same value by construction (both are
-- `MIN(event_date)` per `event_id` upstream) — a true 1:1, zero-skew read —
-- but the planner's cross-axis Form B derivation only registers a
-- *nonzero* margin (a same-name, same-axis zero margin is derived
-- separately). The extra `first_seen_date` filter below restates the
-- tautology as an explicit, conservative 1-day bound so
-- `silver.events_deduped`'s read stays partition-pruned rather than
-- falling back to an unbounded scan.
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
    s.utm_campaign AS session_utm_campaign,
    sc.session_id AS session_id_chained,
    sc.utm_campaign AS session_utm_campaign_chained
FROM smelt.silver.events_deduped e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
JOIN smelt.silver.sessions_chained sc
    ON e.device_id = sc.device_id
   AND e.event_ts >= sc.session_start
   AND e.event_ts <= sc.session_end
-- Form B: the sessions session-cap composition described above.
WHERE s.session_start_date
    BETWEEN e.event_date - INTERVAL '1 day'
        AND e.event_date + INTERVAL '1 day'
-- Form B: the sessions_chained session-cap composition described above.
  AND sc.session_start_date
      BETWEEN e.event_date - INTERVAL '2 days'
          AND e.event_date + INTERVAL '2 days'
-- Form B: the events_deduped tautology described above.
  AND e.first_seen_date
      BETWEEN e.event_date - INTERVAL '1 day'
          AND e.event_date + INTERVAL '1 day'
