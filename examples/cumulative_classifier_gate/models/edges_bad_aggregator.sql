---
materialization: table
refresh: incremental
grain: key
---
-- STRING_AGG is NOT in the keyed allowlist; the classifier must refuse
-- this model (KeyedUnknownCombiner) regardless of run window.
SELECT
    device_id,
    STRING_AGG(CAST(amount AS VARCHAR), ',') AS amounts
FROM smelt.events_ts
GROUP BY device_id
