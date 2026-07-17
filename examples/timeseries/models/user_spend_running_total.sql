---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: spend_date
  partition_column: spend_date
  granularity: day
---
-- A downstream **keyed** model whose driving source is the composed
-- `user_daily_spend` model's own output — not a declared `sources:` entry
-- (`docs/specs/incremental_models.md` §"Key temporal locality (the
-- time-partitioned output)" — "The output as a clocked source": "a
-- downstream keyed model may take it as its clocked driving source"). This
-- model's own `(user_id, spend_date)` GROUP BY makes `spend_date` (the
-- partition column) itself a `unique_key` column, admitting route 1
-- (key-embedded) exactly as `user_daily_spend` admits its own route 1 —
-- the clock propagates window-forward through the composed stage instead
-- of stopping at it (`docs/plans/20260715-composed-axes-conditional-
-- maintenance.md` Phase A5).
SELECT
    user_id,
    spend_date,
    SUM(total_amount) AS running_total
FROM smelt.user_daily_spend
GROUP BY 1, 2
