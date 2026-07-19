---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: spend_date
  partition_column: spend_date
  granularity: day
---
-- Per-(user_id, spend_date) keyed aggregate — the composed shape
-- (`docs/specs/incremental_models.md` §"Key temporal locality (the
-- time-partitioned output)"): a key-addressed output that also carries a
-- `timeseries:` partition column, admitted because `spend_date` (the
-- partition column) is itself a `unique_key` column — key temporal
-- locality's **route 1** ("key-embedded"). Each stored row's partition
-- value is its own key's value, so a `merge_into` target scan can be
-- pruned to the run window (widened by the driving source's derived read
-- margin — zero here, since this model reads no lookback window) instead
-- of scanning the whole table.
SELECT
    user_id,
    CAST(transaction_timestamp AS DATE) AS spend_date,
    SUM(amount) AS total_amount
FROM smelt.sources.raw.transactions
GROUP BY 1, 2
