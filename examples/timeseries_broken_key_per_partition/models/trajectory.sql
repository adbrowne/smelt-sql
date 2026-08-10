---
materialization: table
refresh: incremental
unique_key: [device_id, event_date]
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- A clock plus an identity with partition_column ∈ unique_key derives the
-- `key_per_partition` grain (one row per `(key, partition)`), which is not
-- yet supported by maintenance-plan derivation
-- (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A0):
-- refused fail-loud with `MaintenanceUnsupportedGrain`, never silently
-- collapsed into an ordinary keyed plan with an empty unique_key.
SELECT device_id, event_date, COUNT(*) AS event_count
FROM some_events
GROUP BY device_id, event_date
