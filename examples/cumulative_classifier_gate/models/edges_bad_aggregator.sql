---
materialization: table
refresh: cumulative
---
-- STRING_AGG is NOT in the cumulative allowlist; the classifier must refuse
-- this model (CumulativeUnknownAggregator) regardless of run window.
SELECT
    device_id,
    STRING_AGG(CAST(amount AS VARCHAR), ',') AS amounts
FROM smelt.events_ts
GROUP BY device_id
