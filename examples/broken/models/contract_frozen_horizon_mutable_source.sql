---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      contract_mutable_orders:
        allow_full_scan: true
contract:
  frozen_horizon: '90 days'
---
-- `contract_mutable_orders` is a `mutation_profile: mutable_snapshot` driving
-- source, so the frozen-band late-arrival probe's row-count comparison is
-- blind: refuses with `ContractFrozenHorizonInvalid`
-- (`docs/specs/incremental_models.md` §"The contract lattice").
SELECT
    o.order_date,
    SUM(o.amount) AS total
FROM smelt.sources.contract_mutable_orders o
GROUP BY 1
