---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
contract:
  deferral: '6 hours'
---
-- Demonstrates `contract.deferral`: the maintained state may lag its inputs
-- by up to 6 hours before the `ContractDeferralExceeded` probe fires
-- (`docs/specs/incremental_models.md` §"Contract relaxations (`contract:`)").
SELECT
    date_trunc('day', event_timestamp) as event_date,
    COUNT(*) as event_count
FROM smelt.sources.raw.events
GROUP BY 1
