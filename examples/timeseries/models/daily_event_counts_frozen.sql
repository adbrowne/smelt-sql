---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
contract:
  frozen_horizon: '365 days'
---
-- Demonstrates `contract.frozen_horizon`: partitions older than 365 days
-- are never revisited by maintenance, and the write-eligible range narrows
-- to `end - 365 days` on any run request wider than that
-- (`docs/specs/incremental_models.md` §"Contract relaxations (`contract:`)").
SELECT
    date_trunc('day', event_timestamp) as event_date,
    COUNT(*) as event_count
FROM smelt.sources.raw.events
GROUP BY 1
