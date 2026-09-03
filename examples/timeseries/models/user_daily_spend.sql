---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: spend_date
  partition_column: spend_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      raw.transactions:
        # `raw.transactions` is append-only and folds into this model's own
        # keyed SUM via `Trigger::NewData` — but a transaction that lands
        # late, after this key's day has already been processed by a prior
        # run, is only picked up if something re-derives it: the mutation
        # cell this model now (correctly) derives for it has no statically
        # derivable scan bound (`phases/19-plan.md`,
        # `docs/outcomes/20260815-definition-delta-migrate`). Accepted here
        # as a full-table op rather than bounding it.
        allow_full_scan: true
---
-- Per-(user_id, spend_date) keyed aggregate — the composed shape
-- (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
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
