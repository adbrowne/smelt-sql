---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---
-- The `maintenance_enrichment` join has no clock and no `allow_full_scan`
-- acceptance, so the mutation-sensitive `enrichment_category` group cannot
-- be partition-bounded: refuses with `MaintenanceScanUnbounded`
-- (`docs/specs/incremental_models.md` §Semantics "Partition-local maintenance
-- (the K8 guardrail)").
SELECT
    o.order_id,
    o.order_date,
    e.category AS enrichment_category
FROM smelt.sources.maintenance_orders o
JOIN smelt.sources.maintenance_enrichment e ON o.customer_id = e.customer_id
