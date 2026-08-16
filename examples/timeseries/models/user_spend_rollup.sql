---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: spend_date
  partition_column: spend_date
  granularity: day
---
-- Downstream of the composed `user_daily_spend` model (`grain: key` +
-- `timeseries:`, admitted via key temporal locality's route 1 — see that
-- model's own comment). `incremental_shapes.md` §"Key temporal locality
-- (the time-partitioned output)" — "The output as a clocked source": an
-- admitted composed output is visible to the rest of the DAG exactly like a
-- declared source, so this ordinary `grain: partition` model gets the same
-- source-filter pushdown against it that it would get against a declared
-- `timeseries:` source — the clock propagates through the composed stage
-- instead of stopping there (the "clock-sink" `incremental_models.md`
-- warns a keyed stage can otherwise become).
--
-- The WHERE clause's literal `INTERVAL` lookback (Form B: a filter
-- comparing `spend_date` against itself minus a literal interval) is what
-- statically derives the nonzero pushdown margin below — the same
-- construct `user_daily_spend` and the locality gate's own route-1 tests
-- use to prove a derived margin.
SELECT
    spend_date,
    user_id,
    total_amount
FROM smelt.user_daily_spend
WHERE spend_date >= CAST(spend_date AS DATE) - INTERVAL '3 days'
