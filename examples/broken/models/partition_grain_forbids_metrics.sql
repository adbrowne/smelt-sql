---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---
-- A `grain: partition` model's body calls `smelt.metric(...)` — refused with
-- `PartitionGrainForbidsMetrics` (`docs/specs/incremental_shapes.md`
-- §"Functions inside partition-grain bodies"): the composition of metric
-- expansion with time-filter injection is deliberately unspecified.
SELECT
    o.order_date,
    smelt.metric('revenue') AS revenue
FROM smelt.sources.maintenance_orders o
