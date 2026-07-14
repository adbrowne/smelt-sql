---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Bronze passthrough — provides a named model for downstream silver
-- transformations to attach to instead of binding directly to the raw source.
-- Declares its own time dimension over the (now-typed) event_date partition
-- column so silver/events_parsed's late-arrival acceptance filter can derive
-- a genuine per-source reach from `smelt.bronze.raw_events`, instead of
-- falling back to an unbounded lookup read.
--
-- `materialization: table` (rather than the project default `view`) so
-- `silver.events_parsed`'s `QUALIFY ROW_NUMBER() OVER (PARTITION BY
-- event_id …)` window sees a real base table, not an inlined view
-- definition — DuckDB 1.5.0's binder mis-resolves a window function's
-- column type when the immediate FROM is a view whose own SELECT list
-- carries type-conforming CASTs.
SELECT
    event_id,
    device_id,
    user_id,
    seconds_in_day,
    CAST(event_time AS TIMESTAMP) AS event_time,
    CAST(arrival_time AS TIMESTAMP) AS arrival_time,
    utm_campaign,
    payload,
    CAST(event_date AS DATE) AS event_date
FROM smelt.sources.raw.events
