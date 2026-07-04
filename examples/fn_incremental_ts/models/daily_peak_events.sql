---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
refresh: batched
---

-- D1 scenario A: smelt.define predicates used in WHERE/SELECT of an incremental model.
-- Function call paths: smelt.functions.<fn_name> (directory only, no file stem).
-- The framework injects WHERE event_date >= start AND event_date < end at the
-- outer SELECT; function splices must survive that injection without corruption.
SELECT
    event_date,
    CAST(smelt.functions.hour_bucket(event_ts) AS VARCHAR) AS bucket,
    COUNT(*) AS event_count
FROM smelt.sources.events
WHERE smelt.functions.is_peak_hour(event_ts)
GROUP BY 1, 2
