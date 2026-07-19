---
materialization: table
refresh: incremental
grain: key_per_partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- `grain: key_per_partition` (one row per `(key, partition)`) is not yet
-- supported by maintenance-plan derivation
-- (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A0):
-- refused fail-loud with `MaintenanceUnsupportedGrain`, never silently
-- collapsed into an ordinary keyed plan with an empty unique_key.
SELECT device_id, event_date, COUNT(*) AS event_count
FROM some_events
GROUP BY device_id, event_date
